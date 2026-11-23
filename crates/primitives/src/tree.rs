//! A tree (reka's `Tree`): rows of a hierarchy, opened and closed, chosen
//! one or several at a time (research §9, Tree). No shadcn component uses
//! it; the tests here are its users.
//!
//! The data is the caller's: the roots, how to reach a node's children
//! (`None` for a leaf; an empty list is still a node with children, as
//! reka counts it, `TreeRoot.vue:141,150`) and each node's key. The tree
//! lays the open part out as rows ([`Tree::rows`]) and keeps, by key, what
//! is chosen ([`Choice`] of keys: one or several) and what is open.
//!
//! The data is read tracked, as reka's `items` prop is reactive: roots or
//! children that read signals show a change (a file added, a folder
//! emptied) without the tree being made again, so the highlight and the
//! Tab stop stay on their keys (the stop goes back to the first row when
//! its node leaves), and half-chosen parents follow the new shape.
//!
//! # Millions of rows
//!
//! The rows are worked out once each time what is open changes (a
//! computed, walked without recursion, so a tree of any depth lays out),
//! and found by key in O(1); a key press, a click or a focus change reads
//! them, never walks the data again. Half-chosen parents are the
//! ancestors of what is chosen, found up a parent map made when first
//! asked and again only when the data changes. A tree
//! laid on a virtual list ([`Tree::virtual_list`]) builds only the rows
//! in view, and the keys reach rows never built: the row they reach is
//! kept built and takes focus (reka's `TreeVirtualizer`).
//!
//! # Keys (focus on a row; roving: the tree is one Tab stop)
//!
//! | Key | What it does |
//! | --- | --- |
//! | ArrowDown / ArrowUp | the next / previous usable row; no wrap |
//! | Home / End, PageUp / PageDown | the first / last usable row |
//! | ArrowRight (RTL: ArrowLeft) | closed with children: opens it; open: its first child; a leaf: nothing |
//! | ArrowLeft (RTL: ArrowRight) | open: closes it; otherwise: its parent |
//! | Enter / Space | chooses the row as the tree chooses (toggle, or replace) |
//! | Shift + ArrowUp / ArrowDown | several, replace: the range from the anchor; focus goes with its end |
//! | a printable character | typeahead (1000 ms) |
//!
//! No select-all: reka's Tree has none (Control, Alt or Command + A is
//! left to the page).
//!
//! A click on a row chooses it **and** opens or closes it
//! (`TreeItem.vue:200-203`). Keys typed in a field inside a row are the
//! field's: it takes them before the row does.
//!
//! # Choosing several, propagated
//!
//! With [`TreeConfig::propagate`], choosing a node chooses or lets go its
//! descendants, and a parent is chosen once all its children are; a parent
//! some of whose descendants are chosen is half chosen
//! ([`Tree::state_of`], `TreeItem.vue:70-85`).
//!
//! # Where this differs from reka, and why
//!
//! - A Shift range leaves disabled rows out, as a listbox's does: reka's
//!   Tree keeps them in (ADR-0025 §7, one notion of "cannot be used").
//! - Values compare by key, the tree's one equality (reka compares a single
//!   choice by key and copies the item, `useSelectionBehavior.ts:28-35`).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use gui::events::{KeyEvent, NamedKey};
use gui::focus::FocusId;
use gui::list::{ListState, virtual_list_in};
use gui::{Computed, Direction, Div, Element, FocusHandle, IntoElement, Signal, Toggled};

use crate::availability::Availability;
use crate::collection::{Collection, Items};
use crate::direction::ReadDirection;
use crate::keymap::{Arrows, Wrap};
use crate::listbox::{Bound, Choice, ListConfig, ListCore, ListKey, Options, Pick};
use crate::selection::SelectionBehavior;

/// A node's children; `None` for a leaf.
pub type Children<N> = Rc<dyn Fn(&N) -> Option<Vec<N>>>;

/// The roots, read tracked: a closure reading signals shows their change.
pub type Roots<N> = Rc<dyn Fn() -> Vec<N>>;

/// A tree's data: the caller's nodes and how to read them. Each part is
/// read tracked: what it reads changing lays the rows out again.
pub struct TreeData<N, K> {
    pub roots: Roots<N>,
    /// A node's children; `None` for a leaf.
    pub children: Children<N>,
    pub key: Rc<dyn Fn(&N) -> K>,
    /// The text typeahead matches a node by.
    pub label: Rc<dyn Fn(&N) -> String>,
    /// Whether a node can be used.
    pub availability: Rc<dyn Fn(&N) -> Availability>,
}

impl<N: Clone, K> Clone for TreeData<N, K> {
    fn clone(&self) -> Self {
        Self {
            roots: Rc::clone(&self.roots),
            children: Rc::clone(&self.children),
            key: Rc::clone(&self.key),
            label: Rc::clone(&self.label),
            availability: Rc::clone(&self.availability),
        }
    }
}

/// One row of the open part of a tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row<N, K> {
    pub node: N,
    pub key: K,
    /// Its depth, from 1 (`aria-level`).
    pub level: usize,
    /// Its parent's row.
    pub parent: Option<usize>,
    pub has_children: bool,
    pub open: bool,
    /// Its place among its siblings, from 1, and how many they are.
    pub position: (usize, usize),
    pub label: String,
    pub availability: Availability,
}

/// The rows the open part of a tree shows, found by index or by key.
#[derive(Debug)]
pub struct Rows<N, K> {
    rows: Vec<Row<N, K>>,
    index: HashMap<K, usize>,
}

impl<N, K: Clone + Eq + Hash> Rows<N, K> {
    fn new(rows: Vec<Row<N, K>>) -> Self {
        let index = rows.iter().enumerate().map(|(i, r)| (r.key.clone(), i)).collect();
        Self { rows, index }
    }

    /// The row at `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Row<N, K>> {
        self.rows.get(index)
    }

    /// The rows, in display order.
    pub fn iter(&self) -> impl Iterator<Item = &Row<N, K>> {
        self.rows.iter()
    }

    /// How many rows show.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The row of the node keyed `key`, when it shows (O(1)).
    #[must_use]
    pub fn index_of(&self, key: &K) -> Option<usize> {
        self.index.get(key).copied()
    }
}

impl<N, K> Items for Rows<N, K> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn availability(&self, index: usize) -> Availability {
        self.rows.get(index).map_or(Availability::Disabled, |r| r.availability)
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        Cow::Borrowed(self.rows.get(index).map_or("", |r| r.label.as_str()))
    }
}

impl<N, K: Clone + Eq + Hash> Options<K> for Rows<N, K> {
    fn value(&self, index: usize) -> Option<K> {
        self.rows.get(index).map(|r| r.key.clone())
    }

    fn position(&self, value: &K) -> Option<usize> {
        self.index_of(value)
    }
}

/// Rows compared by pointer (a computed's cutoff: a million rows are never
/// compared).
struct Shared<R>(Rc<R>);

impl<R> PartialEq for Shared<R> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// How a tree chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeConfig {
    pub behavior: SelectionBehavior,
    /// Several: choosing a node chooses its descendants, and a parent is
    /// chosen once all its children are (reka's `propagateSelect`).
    pub propagate: bool,
}

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            behavior: SelectionBehavior::Toggle,
            propagate: false,
        }
    }
}

type Veto<K> = gui::Callback<(K, bool), Pick>;

struct State<N: 'static, K: 'static> {
    data: TreeData<N, K>,
    config: TreeConfig,
    /// The rows laid out, by their focus (filled by layout): which row a
    /// focus is of.
    laid: Collection<K>,
    /// The caller's answer to each opening (`true`) and closing.
    veto_open: Option<Veto<K>>,
    /// The virtual list the rows are laid on, while the part that laid
    /// them is mounted ([`Tree::attach_list`]).
    virtual_rows: gui::Holder<ListState>,
}

/// A tree: made once in its root's setup, `Copy`.
pub struct Tree<N: 'static, K: 'static, M: Choice<K>> {
    value: Bound<M>,
    expanded: Bound<Vec<K>>,
    list: ListCore<K>,
    rows: Computed<Shared<Rows<N, K>>>,
    /// The keys some of whose descendants are chosen (propagated, several).
    mixed: Computed<Shared<HashSet<K>>>,
    /// The row that is the tree's Tab stop: the focused one, else the last.
    stop: Signal<Option<K>>,
    /// Each row's focus, by its key: issued as the row is first built, let
    /// go once the row is neither laid out nor the stop nor focused, or
    /// leaves the data (a row added again is focused through a new handle).
    focus: gui::Keyed<K, FocusHandle>,
    state: Signal<State<N, K>>,
}

impl<N: 'static, K: 'static, M: Choice<K>> Clone for Tree<N, K, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N: 'static, K: 'static, M: Choice<K>> Copy for Tree<N, K, M> {}

impl<N: 'static, K: 'static, M: Choice<K>> std::fmt::Debug for Tree<N, K, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tree").finish_non_exhaustive()
    }
}

impl<N: Clone + 'static, K: Clone + PartialEq + Eq + Hash + 'static, M: Choice<K>> Tree<N, K, M> {
    /// A tree of `data`, holding `value` (keys chosen) and `expanded` (keys
    /// open); made in its root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(
        data: TreeData<N, K>,
        value: Bound<M>,
        expanded: Bound<Vec<K>>,
        config: TreeConfig,
        direction: ReadDirection,
    ) -> Self {
        let list = ListCore::new(
            ListConfig::LISTBOX
                .arrows(Arrows::Vertical)
                .wrap(Wrap::Stop)
                .behavior(config.behavior)
                .direction(direction)
                .select_all(false),
        );
        let state = Signal::new(State {
            data,
            config,
            laid: Collection::new(),
            veto_open: None,
            virtual_rows: gui::Holder::new(),
        });
        let rows = Computed::new(move || {
            let open: HashSet<K> = expanded.get().into_iter().collect();
            let data = state.with_untracked(|s| s.data.clone());
            Shared(Rc::new(Rows::new(flatten(&data, &open))))
        });
        // Each node's parent's key, over the whole tree: worked out when a
        // half-chosen state is first asked, and again as the data changes.
        let parents = Computed::new(move || {
            let data = state.with_untracked(|s| s.data.clone());
            Shared(Rc::new(parents_of(&data)))
        });
        let mixed = Computed::new(move || {
            let value = value.get();
            let propagate = state.with_untracked(|s| s.config.propagate);
            let mut mixed = HashSet::new();
            if M::MULTIPLE && propagate {
                let parents = parents.with(|p| Rc::clone(&p.0));
                for k in value.values() {
                    let mut at = parents.get(&k);
                    while let Some(p) = at {
                        if !mixed.insert(p.clone()) {
                            break;
                        }
                        at = parents.get(p);
                    }
                }
            }
            Shared(Rc::new(mixed))
        });
        let me = Self {
            value,
            expanded,
            list,
            rows,
            mixed,
            stop: Signal::new(None),
            focus: gui::Keyed::new(),
            state,
        };
        // A row no longer laid out, nor the stop, nor focused, or gone from
        // the data, lets go of its focus.
        me.state.with_untracked(|s| s.laid.clone()).on_laid_out(move |now| {
            if !me.state.is_alive() || me.focus.is_empty() {
                return;
            }
            let rows = me.rows_untracked();
            let stop = me.stop.get_untracked();
            let focused = gui::untracked(gui::focus::focused);
            let laid: HashSet<FocusId> = now.iter().map(|i| i.focus.id()).collect();
            me.focus.retain(|k, h| {
                rows.index_of(k).is_some()
                    && (laid.contains(&h.id()) || stop.as_ref() == Some(k) || focused == Some(h.id()))
            });
        });
        // The highlight is the focus: a row focused (by Tab, the pointer,
        // the keys) is highlighted and is the tree's stop.
        gui::watch(gui::focus::focused, move |now, _| {
            if !me.state.is_alive() {
                return;
            }
            let key = now.and_then(|f| {
                me.state
                    .with_untracked(|s| s.laid.index_of(f).and_then(|i| s.laid.value(i)))
            });
            match key {
                Some(k) => {
                    let rows = me.rows_untracked();
                    if let Some(i) = rows.index_of(&k) {
                        me.list.highlight_at(i, k.clone());
                    }
                    me.stop.set(Some(k));
                }
                None => me.list.clear(),
            }
        });
        me
    }

    /// Ask `f` before each choice.
    pub fn on_pick(self, f: impl Into<gui::Callback<K, Pick>>) {
        self.list.on_pick(f.into());
    }

    /// Ask `f` before each opening (`true`) and closing of a node.
    pub fn on_toggle(self, f: impl Into<gui::Callback<(K, bool), Pick>>) {
        let f: Veto<K> = f.into();
        self.state.update(|s| s.veto_open = Some(f));
    }

    /// The rows of the open part (tracked: reads what is open). Worked out
    /// once per change of what is open, shared after that.
    #[must_use]
    pub fn rows(self) -> Rc<Rows<N, K>> {
        self.rows.with(|r| Rc::clone(&r.0))
    }

    fn rows_untracked(self) -> Rc<Rows<N, K>> {
        gui::untracked(|| self.rows())
    }

    /// Whether the node of `row` is chosen, half chosen or not (tracked):
    /// with propagation a parent some of whose descendants are chosen is
    /// half chosen. O(1).
    #[must_use]
    pub fn state_of(self, row: &Row<N, K>) -> Toggled {
        if self.value.get().holds(&row.key) {
            return Toggled::True;
        }
        if row.has_children && self.mixed.with(|m| m.0.contains(&row.key)) {
            return Toggled::Mixed;
        }
        Toggled::False
    }

    /// Whether the row keyed `key` holds focus (tracked): the highlight.
    #[must_use]
    pub fn is_highlighted(self, key: &K) -> bool {
        self.list.is_highlighted(key)
    }

    fn focus_of(self, key: &K) -> FocusHandle {
        self.focus.get(key, FocusHandle::new)
    }

    /// How many rows the tree keeps a focus for: those laid out, the stop
    /// and the focused one (each let go as its row is not, or leaves the
    /// data).
    #[must_use]
    pub fn kept_focus(self) -> usize {
        self.focus.len().max(self.state.with_untracked(|s| s.laid.len()))
    }

    /// Open or close the row at `index` of `rows`, unless the caller refuses.
    pub fn toggle(self, rows: &Rows<N, K>, index: usize) {
        let Some(row) = rows.get(index) else { return };
        if !row.has_children {
            return;
        }
        let open = !row.open;
        let veto = self.state.with_untracked(|s| s.veto_open.clone());
        if veto.is_some_and(|f| f.call((row.key.clone(), open)) == Pick::Refuse) {
            return;
        }
        let key = row.key.clone();
        if !open
            && let Some(stop) = self.stop.get_untracked()
            && let Some(at) = rows.index_of(&stop)
            && is_below(rows, at, index)
        {
            // The stop goes up to a row that stays shown.
            self.stop.set(Some(key.clone()));
        }
        self.expanded.update(|e| {
            if open {
                e.push(key);
            } else {
                e.retain(|k| *k != key);
            }
        });
    }

    /// Choose the row at `index` as the tree chooses, propagating when it
    /// says so.
    pub fn choose(self, rows: &Rows<N, K>, index: usize) -> ListKey {
        let was = self.value.get_untracked();
        let key = self.list.choose(rows, index, Some(self.value));
        if matches!(key, ListKey::Chose(_)) && M::MULTIPLE && self.state.with_untracked(|s| s.config.propagate) {
            self.propagate(rows, index, &was);
        }
        key
    }

    fn propagate(self, rows: &Rows<N, K>, index: usize, was: &M) {
        let Some(row) = rows.get(index) else { return };
        let data = self.state.with_untracked(|s| s.data.clone());
        let on = !was.holds(&row.key);
        let below: HashSet<K> = descendants(&data, &row.node).into_iter().collect();
        self.value.update(|m| {
            let mut all = m.values();
            let mut held: HashSet<K> = all.iter().cloned().collect();
            if on {
                // In the tree's order, after what was chosen.
                for k in descendants(&data, &row.node) {
                    if held.insert(k.clone()) {
                        all.push(k);
                    }
                }
            } else {
                all.retain(|k| !below.contains(k));
                held.retain(|k| !below.contains(k));
            }
            // A parent is chosen once all its children are, up the rows.
            let mut at = row.parent;
            while let Some(p) = at {
                let Some(parent) = rows.get(p) else { break };
                let kids = (data.children)(&parent.node).unwrap_or_default();
                let full = !kids.is_empty() && kids.iter().all(|k| held.contains(&(data.key)(k)));
                let has = held.contains(&parent.key);
                if full && !has {
                    held.insert(parent.key.clone());
                    all.push(parent.key.clone());
                } else if !full && has {
                    held.remove(&parent.key);
                    all.retain(|k| *k != parent.key);
                }
                at = parent.parent;
            }
            m.hold(all);
        });
    }

    /// The keys of the row at `index` of `rows` (the module's table):
    /// whether one was taken.
    pub fn key(self, rows: &Rows<N, K>, index: usize, ev: &KeyEvent) -> bool {
        let Some(row) = rows.get(index) else { return false };
        let rtl = self.list.config().direction.get() == Direction::Rtl;
        let (inward, outward) = if rtl {
            (NamedKey::ArrowLeft, NamedKey::ArrowRight)
        } else {
            (NamedKey::ArrowRight, NamedKey::ArrowLeft)
        };
        let plain = !(ev.modifiers.control || ev.modifiers.alt || ev.modifiers.command || ev.modifiers.shift);
        if plain && ev.is(inward) {
            if row.has_children && !row.open {
                self.toggle(rows, index);
            } else if row.has_children
                && rows
                    .get(index.saturating_add(1))
                    .is_some_and(|c| c.parent == Some(index))
            {
                self.focus_row(rows, index.saturating_add(1));
            }
            return true;
        }
        if plain && ev.is(outward) {
            if row.has_children && row.open {
                self.toggle(rows, index);
            } else if let Some(p) = row.parent {
                self.focus_row(rows, p);
            }
            return true;
        }
        // Enter and Space choose as a click does (reka's `handleSelect` ->
        // `rootContext.onSelect` either way), so they propagate too.
        let propagating = M::MULTIPLE && self.state.with_untracked(|s| s.config.propagate);
        let was = propagating.then(|| self.value.get_untracked());
        let key = self.list.key(ev, rows, Some(self.value));
        if let (ListKey::Chose(at), Some(was)) = (&key, was.as_ref()) {
            self.propagate(rows, *at, was);
        }
        // The highlight is the focus: a move, and a Shift range's end, take
        // it (rows have no focus of their own in `Rows`, so the core moved
        // only the highlight).
        if matches!(key, ListKey::Highlighted(_) | ListKey::ChoseMany)
            && let Some(i) = self.list.index_in(rows)
        {
            self.focus_row(rows, i);
        }
        key.consumed()
    }

    /// Focus the row at `index`: on a virtual list it is brought into view
    /// and kept built first, and takes focus once laid out.
    fn focus_row(self, rows: &Rows<N, K>, index: usize) {
        let Some(row) = rows.get(index) else { return };
        self.list.highlight_at(index, row.key.clone());
        self.stop.set(Some(row.key.clone()));
        let focus = self.focus_of(&row.key);
        let list = self.state.with_untracked(|s| s.virtual_rows).get_untracked();
        match list {
            Some(state) => {
                let row = u64::try_from(index).unwrap_or(u64::MAX);
                state.set_active(Some(row));
                state.scroll_into_view(row);
                gui::after_layout(move || gui::focus::set_focus(Some(focus.id()), true));
            }
            None => gui::focus::set_focus(Some(focus.id()), true),
        }
    }

    /// `d` as the tree (`role="tree"`, multi-select when it holds several).
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(gui::Role::Tree).aria_multiselectable(M::MULTIPLE)
    }

    /// Lay the rows on the virtual list `state` while the calling setup's
    /// part is mounted: the keys bring a row scrolled away into view
    /// through it, and the highlighted row is its active one, kept built.
    /// Called in the setup of the part whose render calls
    /// [`Tree::virtual_list`]; its leaving lets go of the list.
    #[track_caller]
    pub fn attach_list(self, state: &ListState) {
        self.state.with_untracked(|s| s.virtual_rows).hold(state.clone());
        state.active_from(move || {
            if !self.rows.is_alive() {
                return None;
            }
            let rows = self.rows();
            self.list.index_tracked(&*rows).and_then(|i| u64::try_from(i).ok())
        });
    }

    /// The rows laid on the virtual list `state` (attached in the part's
    /// setup, [`Tree::attach_list`]), `build` making each row's look (given
    /// to [`Tree::row`]): only the rows in view are built, the highlighted
    /// one kept built while scrolled away, and the keys reach rows never
    /// built. Called in the part's render; `rows` is what it read
    /// ([`Tree::rows`]).
    #[must_use]
    pub fn virtual_list(
        self,
        state: &ListState,
        rows: &Rc<Rows<N, K>>,
        build: impl Fn(&Row<N, K>, usize) -> Div,
    ) -> Div {
        let n = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        let key = |i: u64| {
            rows.get(usize::try_from(i).unwrap_or(usize::MAX))
                .map_or(0, |r| self.focus_of(&r.key).id().key())
        };
        let row = |i: u64| -> Element {
            let i = usize::try_from(i).unwrap_or(usize::MAX);
            match rows.get(i) {
                Some(r) => self.row(build(r, i), rows, i).into_element(),
                None => Element::Empty,
            }
        };
        let list = virtual_list_in(state, n, key, row, |v| v);
        self.root(list)
    }

    /// `d` as the row at `index` of `rows`: a tree item to assistive
    /// technology (selected, open while it has children, its level and its
    /// place among its siblings), focusable, the tree's one Tab stop when it
    /// is the focused or last focused row (else the first); it takes the
    /// tree's keys, and a click chooses it and opens or closes it. `rows` is
    /// what the look's render read ([`Tree::rows`]).
    #[must_use]
    pub fn row(self, d: Div, rows: &Rows<N, K>, index: usize) -> Div {
        let Some(row) = rows.get(index) else { return d };
        let focus = self.focus_of(&row.key);
        let laid = self.state.with_untracked(|s| s.laid.clone());
        let d = laid.item(d, crate::collection::Item::new(row.key.clone(), focus));
        let chosen = self.value.get().holds(&row.key);
        let first = crate::keymap::step(rows, None, crate::keymap::Intent::First, Wrap::Stop)
            .and_then(|i| rows.get(i))
            .map(|r| r.key.clone());
        // The stop is read when Tab looks: moving focus renders no row. A
        // stop whose node left the data gives way to the first row.
        let (tree, mine) = (self, row.key.clone());
        let is_stop = move || {
            tree.stop.is_alive()
                && tree
                    .stop
                    .get_untracked()
                    .filter(|k| tree.rows_untracked().index_of(k).is_some())
                    .map_or(first.as_ref() == Some(&mine), |k| k == mine)
        };
        let d = row
            .availability
            .apply(d)
            .role(gui::Role::TreeItem)
            .aria_selected(chosen)
            .aria_level(row.level)
            .aria_pos_in_set(row.position.0, row.position.1)
            .track_focus(focus)
            .tab_stop(is_stop);
        let d = if row.has_children { d.aria_expanded(row.open) } else { d };
        if !row.availability.focusable() {
            return d;
        }
        let key = row.key.clone();
        let id: FocusId = focus.id();
        let me = self;
        let watch_key = key.clone();
        d.on_key_down(move |ev| {
            if gui::untracked(gui::focus::focused) != Some(id) {
                return false;
            }
            let rows = me.rows_untracked();
            match rows.index_of(&watch_key) {
                Some(i) => me.key(&rows, i, ev),
                None => false,
            }
        })
        .on_click(move |_| {
            let rows = me.rows_untracked();
            if let Some(i) = rows.index_of(&key) {
                me.focus_row(&rows, i);
                me.choose(&rows, i);
                me.toggle(&rows, i);
            }
        })
    }
}

/// Whether the row at `at` lies below (is a descendant of) the row at `of`.
fn is_below<N, K>(rows: &Rows<N, K>, at: usize, of: usize) -> bool {
    let mut p = rows.rows.get(at).and_then(|r| r.parent);
    while let Some(i) = p {
        if i == of {
            return true;
        }
        p = rows.rows.get(i).and_then(|r| r.parent);
    }
    false
}

/// The open part of `data` as rows, depth first, without recursion.
fn flatten<N: Clone, K: Clone + Eq + Hash>(data: &TreeData<N, K>, open: &HashSet<K>) -> Vec<Row<N, K>> {
    let mut rows = Vec::new();
    // Each level's nodes left to walk, reversed so the next is at the end,
    // with that level's depth, parent row and size.
    let mut first = (data.roots)();
    first.reverse();
    let count = first.len();
    let mut stack: Vec<(Vec<N>, usize, Option<usize>, usize)> = vec![(first, 1, None, count)];
    while let Some((nodes, level, parent, count)) = stack.last_mut() {
        let (level, parent, count) = (*level, *parent, *count);
        let Some(node) = nodes.pop() else {
            stack.pop();
            continue;
        };
        let place = count.saturating_sub(nodes.len());
        let key = (data.key)(&node);
        let children = (data.children)(&node);
        let is_open = children.is_some() && open.contains(&key);
        let at = rows.len();
        rows.push(Row {
            label: (data.label)(&node),
            availability: (data.availability)(&node),
            node,
            key,
            level,
            parent,
            has_children: children.is_some(),
            open: is_open,
            position: (place, count),
        });
        if is_open && let Some(mut kids) = children {
            kids.reverse();
            let n = kids.len();
            stack.push((kids, level.saturating_add(1), Some(at), n));
        }
    }
    rows
}

/// The keys below `node`, in the tree's order, without recursion.
fn descendants<N, K>(data: &TreeData<N, K>, node: &N) -> Vec<K> {
    let mut out = Vec::new();
    let mut stack: Vec<N> = (data.children)(node).unwrap_or_default();
    stack.reverse();
    while let Some(n) = stack.pop() {
        out.push((data.key)(&n));
        if let Some(mut kids) = (data.children)(&n) {
            kids.reverse();
            stack.extend(kids);
        }
    }
    out
}

/// Each node's parent, over the whole tree, without recursion.
fn parents_of<N: Clone, K: Clone + Eq + Hash>(data: &TreeData<N, K>) -> HashMap<K, K> {
    let mut parents = HashMap::new();
    let mut stack: Vec<N> = (data.roots)();
    while let Some(n) = stack.pop() {
        if let Some(kids) = (data.children)(&n) {
            let k = (data.key)(&n);
            for kid in kids {
                parents.insert((data.key)(&kid), k.clone());
                stack.push(kid);
            }
        }
    }
    parents
}
