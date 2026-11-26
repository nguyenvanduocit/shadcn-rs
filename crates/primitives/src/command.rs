//! shadcn's command (`ui/command`, on reka's `Listbox` and `ListboxFilter`):
//! a search field over a list of sectioned items, filtered as the user
//! types, highlighted by the pointer and the arrow keys, picked with a click
//! or Enter (research §9, Listbox; shadcn's `Command.vue`). shadcn's
//! combobox lists its options the same way ([`Command::new`] takes its
//! term).
//!
//! # The data
//!
//! The list is data: [`CommandSection`]s of [`CommandItem`]s. Under a
//! search it lays out as rows ([`flatten`], [`Flat`]): a group's heading,
//! its items, separators, the empty state. An item shows when its value
//! contains the search, case and accents aside (reka's `useFilter`,
//! [`crate::filter`]); a group shows while one of its items does; the empty
//! state shows when a search leaves no item (`CommandItem.vue:22-37`,
//! `CommandGroup.vue:20,38`, `CommandEmpty.vue:15`). The rows sit on a
//! virtual list, so a palette of a million items filters and walks as one
//! of ten: the keys run on the data ([`Flat`] is [`Options`] by index), and
//! the highlighted row stays built (`gui::list::ListState::set_active`).
//!
//! # The highlight
//!
//! The field keeps focus; the highlighted item is its active descendant.
//! With nothing highlighted the first item is (reka highlights the first
//! item on mount and on every change of the search, `ListboxFilter.vue:75-80`).
//! The pointer highlights what it moves over (`highlightOnHover`, on in
//! shadcn's command). Picking an item empties the search
//! (`CommandItem.vue:71-73`).
//!
//! # Keys (in the field)
//!
//! ArrowDown / ArrowUp: the next / previous item the search leaves,
//! stopping at the ends; Home / End (and the page keys): the first / last;
//! Enter: picks the highlighted item (Control, Alt or Command with it is
//! the form's). Disabled items are skipped. Everything else is the field's.
//! While an input method composes in the field, every key is the input
//! method's: the arrows, Home, End and Enter neither move the highlight nor
//! pick (research §9, Listbox filter).
//!
//! # Where this differs from reka and shadcn-vue, and why
//!
//! - The search field is a combobox to assistive technology, expanded,
//!   controlling the list and naming the highlighted item; reka's filter
//!   is a bare text box, so a screen reader does not know which list it
//!   drives (research §4 item 14).
//! - A disabled item is dimmed and its pick does not run; shadcn-vue's look
//!   reads a disabled state reka never writes (`CommandItem.vue:70,82`),
//!   so in Chrome a disabled command item looks usable (research §9,
//!   Listbox "Look"; the dimming is the look's, `verdict/better.tsv`).
//! - An option is told its group by its heading as its description
//!   ([`Command::option`]'s `group`), not by a `role=group` node around the
//!   group's options as reka's `ListboxGroup` is: the rows lie flat on a
//!   virtual list, so no node holds a group's options, and a relation to a
//!   heading scrolled out of view would not be told (ADR-0025, "Group 5:
//!   second review fixes").

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use gui::element::Node;
use gui::events::{KeyEvent, NamedKey};
use gui::list::ListState;
use gui::{Callback, Computed, Div, ElementKey, Members, NodeRef, Signal, TextField, key_of, watch};

use crate::availability::Availability;
use crate::collection::{Items, step_in_list};
use crate::filter::Filter;
use crate::keymap::Keymap;
use crate::listbox::{ListConfig, ListCore, ListFocus, Options};
use crate::roving::Nav as RovingNav;

/// An item of a command list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandItem {
    /// What the search matches and what picking it hands over.
    pub value: String,
    /// Skipped by the keys, not picked.
    pub disabled: bool,
    /// Valued `null` (`<ComboboxItem :value="null">`, a "Create …" row): its
    /// `value` is only what the search matches.
    pub null: bool,
}

impl CommandItem {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            disabled: false,
            null: false,
        }
    }

    /// An item that cannot be picked.
    #[must_use]
    pub fn disabled(value: impl Into<String>) -> Self {
        Self {
            disabled: true,
            ..Self::new(value)
        }
    }

    /// An item valued `null`, searched as `label`.
    #[must_use]
    pub fn null(label: impl Into<String>) -> Self {
        Self {
            null: true,
            ..Self::new(label)
        }
    }
}

/// A part of a command list, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandSection {
    /// A group under an optional heading; hidden when the search hides all
    /// its items.
    Group {
        heading: Option<String>,
        items: Vec<CommandItem>,
    },
    /// Items straight in the list, no group around them (reka's `ListboxContent`).
    Items(Vec<CommandItem>),
    /// A separator, always shown.
    Separator,
    /// The empty state, shown when no item shows.
    Empty(String),
}

impl CommandSection {
    /// A group of `items` under `heading`.
    #[must_use]
    pub fn group(heading: Option<&str>, items: impl IntoIterator<Item = CommandItem>) -> Self {
        Self::Group {
            heading: heading.map(str::to_owned),
            items: items.into_iter().collect(),
        }
    }

    /// The empty state, saying `message`.
    #[must_use]
    pub fn empty(message: &str) -> Self {
        Self::Empty(message.to_owned())
    }

    /// Its items (none for a separator or the empty state).
    #[must_use]
    pub fn items(&self) -> &[CommandItem] {
        match self {
            Self::Group { items, .. } | Self::Items(items) => items,
            Self::Separator | Self::Empty(_) => &[],
        }
    }

    /// Its heading, for a group that has one.
    #[must_use]
    pub fn heading(&self) -> Option<&str> {
        match self {
            Self::Group { heading, .. } => heading.as_deref(),
            _ => None,
        }
    }
}

/// A row of the list as laid out: one per heading, item, separator and
/// empty state the search leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Heading {
        section: usize,
    },
    /// `first`/`last`: the first and last rows of its group.
    Item {
        section: usize,
        item: usize,
        first: bool,
        last: bool,
    },
    Separator {
        section: usize,
    },
    Empty {
        section: usize,
    },
}

/// An item the keys reach: where it is in the data (its section, its
/// place there, whether in the footer), and the row it is on (`None` in a
/// footer, which is not virtual).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    pub section: u32,
    pub item: u32,
    pub footer: bool,
    pub row: Option<u64>,
}

/// What a list shows under the current search: its rows, its footer's,
/// and the items the keys reach. Compared by pointer, so a million rows
/// are never compared.
#[derive(Clone)]
pub struct Flat(pub Rc<FlatRows>);

/// See [`Flat`]. It names the items by their place in the data it holds
/// (shared, not copied), so it is made off the UI thread for a long list.
pub struct FlatRows {
    pub rows: Vec<Row>,
    pub tail: Vec<Row>,
    pub reach: Vec<Reach>,
    /// How many of `reach` are in the list, not its footer (they come first).
    main: usize,
    sections: Arc<Vec<CommandSection>>,
    footer: Arc<Vec<CommandSection>>,
}

impl FlatRows {
    /// The item the keys reach at `index`.
    #[must_use]
    pub fn item(&self, index: usize) -> Option<&CommandItem> {
        let r = self.reach.get(index)?;
        let sections = if r.footer { &self.footer } else { &self.sections };
        sections
            .get(usize::try_from(r.section).ok()?)?
            .items()
            .get(usize::try_from(r.item).ok()?)
    }

    /// The key of the list's row `row`: its place in the data and its value.
    #[must_use]
    pub fn key(&self, row: usize) -> ElementKey {
        self.rows.get(row).map_or(0, |r| row_key(&self.sections, r))
    }

    /// The index among the items the keys reach of the option at `set`
    /// (its place among its rows' options, from 1) valued `value`: O(1).
    fn index_at(&self, set: Option<(usize, usize)>, value: &str, footer: bool) -> Option<usize> {
        let at = set.and_then(|(at, _)| at.checked_sub(1))?;
        let i = if footer { self.main.checked_add(at)? } else { at };
        (self.item(i).is_some_and(|it| it.value == value)).then_some(i)
    }
}

impl PartialEq for Flat {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Items for FlatRows {
    fn len(&self) -> usize {
        self.reach.len()
    }

    fn availability(&self, index: usize) -> Availability {
        self.item(index)
            .map_or(Availability::Disabled, |i| Availability::from_disabled(i.disabled))
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        Cow::Borrowed(self.item(index).map_or("", |i| i.value.as_str()))
    }
}

impl Options<String> for FlatRows {
    fn value(&self, index: usize) -> Option<String> {
        self.item(index).map(|i| i.value.clone())
    }
}

/// How many items a search filters in the frame it is typed in; a longer
/// list is filtered off the UI thread ([`Command::list_setup`]), the rows
/// it showed staying until the new ones come. Below it the result is there
/// in the same frame, which a thread's hand-off could not beat.
pub const FILTER_AT_ONCE: usize = 20_000;

/// The rows `sections` (and below them `footer`) show under `search`.
#[must_use]
pub fn flatten(sections: Arc<Vec<CommandSection>>, footer: Arc<Vec<CommandSection>>, search: &str) -> Flat {
    Flat(Rc::new(
        flatten_rows(sections, footer, search, &|| false).unwrap_or_else(unreachable_rows),
    ))
}

/// Never reached: a flatten that is never cancelled finishes.
fn unreachable_rows() -> FlatRows {
    FlatRows {
        rows: Vec::new(),
        tail: Vec::new(),
        reach: Vec::new(),
        main: 0,
        sections: Arc::new(Vec::new()),
        footer: Arc::new(Vec::new()),
    }
}

/// [`flatten`]'s rows, on any thread; `None` once `cancelled` says a newer
/// search made them stale (asked every few thousand items).
fn flatten_rows(
    sections: Arc<Vec<CommandSection>>,
    footer: Arc<Vec<CommandSection>>,
    search: &str,
    cancelled: &dyn Fn() -> bool,
) -> Option<FlatRows> {
    let filter = Filter::default();
    let search = filter.search(search);
    let mut reach = Vec::new();
    let rows = flatten_into(&sections, &search, &mut reach, false, cancelled)?;
    let main = reach.len();
    let tail = flatten_into(&footer, &search, &mut reach, true, cancelled)?;
    Some(FlatRows {
        rows,
        tail,
        reach,
        main,
        sections,
        footer,
    })
}

fn flatten_into(
    sections: &[CommandSection],
    search: &crate::filter::Search<'_>,
    reach: &mut Vec<Reach>,
    footer: bool,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Row>> {
    // Each section's items the search leaves, each item matched once.
    let mut seen = 0_usize;
    let mut shown: Vec<Vec<usize>> = Vec::with_capacity(sections.len());
    for s in sections {
        let mut kept = Vec::new();
        for (k, i) in s.items().iter().enumerate() {
            seen = seen.wrapping_add(1);
            if seen.is_multiple_of(4096) && cancelled() {
                return None;
            }
            if search.contains(&i.value) {
                kept.push(k);
            }
        }
        shown.push(kept);
    }
    let any = shown.iter().any(|k| !k.is_empty());
    let mut rows = Vec::new();
    for ((section, s), kept) in sections.iter().enumerate().zip(&shown) {
        match s {
            CommandSection::Separator => rows.push(Row::Separator { section }),
            CommandSection::Empty(_) => {
                // Only a search can leave nothing (shadcn's `CommandEmpty.vue:15`
                // renders with a search and no match).
                if !any && !search.is_empty() {
                    rows.push(Row::Empty { section });
                }
            }
            CommandSection::Group { .. } | CommandSection::Items(_) => {
                if kept.is_empty() {
                    continue;
                }
                let heading = match s {
                    CommandSection::Group { heading, .. } => heading.as_ref(),
                    _ => None,
                };
                let grouped = matches!(s, CommandSection::Group { .. });
                if grouped && heading.is_some() {
                    rows.push(Row::Heading { section });
                }
                let last_k = kept.len().saturating_sub(1);
                for (k, &item) in kept.iter().enumerate() {
                    let at = u64::try_from(rows.len()).unwrap_or(u64::MAX);
                    reach.push(Reach {
                        section: u32::try_from(section).unwrap_or(u32::MAX),
                        item: u32::try_from(item).unwrap_or(u32::MAX),
                        footer,
                        row: (!footer).then_some(at),
                    });
                    rows.push(Row::Item {
                        section,
                        item,
                        first: grouped && k == 0 && heading.is_none(),
                        last: grouped && k == last_k,
                    });
                }
            }
        }
    }
    Some(rows)
}

/// Each row's place among the item rows, from 1, and how many there are
/// (`aria-posinset`, `aria-setsize`: headings and separators are no options).
#[must_use]
pub fn options_of(rows: &[Row]) -> Vec<Option<(usize, usize)>> {
    let total = rows.iter().filter(|r| matches!(r, Row::Item { .. })).count();
    let mut n = 0_usize;
    rows.iter()
        .map(|r| {
            matches!(r, Row::Item { .. }).then(|| {
                n = n.saturating_add(1);
                (n, total)
            })
        })
        .collect()
}

/// A row's key: its place in the data.
fn row_key(sections: &[CommandSection], r: &Row) -> ElementKey {
    match *r {
        Row::Heading { section } => key_of(&format!("h{section}")),
        Row::Item { section, item, .. } => {
            let value = sections
                .get(section)
                .and_then(|s| s.items().get(item))
                .map_or("", |i| i.value.as_str());
            key_of(&format!("i{section}/{item}/{value}"))
        }
        Row::Separator { section } => key_of(&format!("s{section}")),
        Row::Empty { section } => key_of(&format!("e{section}")),
    }
}

/// One list of a command, as its keys reach it: its rows under the search,
/// the virtual list it lays them on, and what picking an item of it runs.
#[derive(Clone)]
pub struct CommandList {
    /// The rows under the search: set at once for a short list, as they
    /// come from their thread for a long one.
    pub flat: Signal<Flat>,
    pub state: ListState,
    pub on_select: Option<Callback<CommandItem>>,
}

/// What picking an item does at the root.
type OnPick = Callback<CommandItem>;

/// Whether an item is chosen (what its option tells assistive technology).
type Holds = Rc<dyn Fn(&CommandItem) -> bool>;

/// A command: made once in its root's setup, `Copy`.
#[derive(Clone, Copy)]
pub struct Command {
    /// The field the keys come from: while an input method composes in it,
    /// they are the input method's.
    field: TextField,
    /// What filters the list.
    query: Computed<String>,
    list: ListCore<String>,
    lists: Signal<Members<CommandList>>,
    on_pick: Signal<OnPick>,
    holds: Signal<Holds>,
    /// The list's node, which the search field controls.
    node: NodeRef,
}

impl PartialEq for Command {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list
    }
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command").finish_non_exhaustive()
    }
}

/// How a command's list navigates: down, stopping at the ends.
pub const KEYMAP: Keymap = Keymap::LIST;

impl Command {
    /// A command typed in `field` (its search field, or a combobox's),
    /// filtered by `query` (the field's text, or a combobox's term),
    /// highlighting through `list` (its own, or a
    /// combobox's); `holds` says whether an item is chosen (read tracked
    /// in its row's render: a command's value, a combobox's), and
    /// `on_pick` runs at the root with each item picked. Made in its root's
    /// setup.
    #[must_use]
    #[track_caller]
    pub fn new(
        field: TextField,
        query: impl Fn() -> String + 'static,
        list: ListCore<String>,
        holds: impl Fn(&CommandItem) -> bool + 'static,
        on_pick: impl Into<Callback<CommandItem>>,
    ) -> Self {
        let query = Computed::new(query);
        let on_pick: OnPick = on_pick.into();
        let holds: Holds = Rc::new(holds);
        let me = Self {
            field,
            query,
            list,
            lists: Signal::new(Members::new()),
            on_pick: Signal::new(on_pick),
            holds: Signal::new(holds),
            node: NodeRef::new(),
        };
        // A new search highlights its first item.
        watch(move || query.get(), move |_, _| me.list.clear());
        me
    }

    /// A listbox as a command's own: in its field, hovering highlights.
    #[must_use]
    #[track_caller]
    pub fn listbox() -> ListCore<String> {
        ListCore::new(ListConfig::LISTBOX.focus(ListFocus::Field).hover(true))
    }

    /// The search (tracked).
    #[must_use]
    pub fn query(self) -> String {
        self.query.get()
    }

    /// The list's node, which a field or a button controls.
    #[must_use]
    pub fn list_ref(self) -> NodeRef {
        self.node
    }

    /// This command with its list known by `node`: the node its owner
    /// already points at (a combobox's content, which its trigger controls
    /// while open), so one node is named, not two. Called right after
    /// [`Command::new`].
    #[must_use]
    pub fn list_known_as(mut self, node: NodeRef) -> Self {
        self.node = node;
        self
    }

    fn lists(self) -> Members<CommandList> {
        self.lists.with_untracked(Clone::clone)
    }

    /// The command's list as last laid out.
    #[must_use]
    pub fn first_list(self) -> Option<CommandList> {
        self.lists().with(|l| l.first().cloned())
    }

    /// The highlighted item's index among `flat`'s, tracked: the highlight,
    /// else the first item.
    #[must_use]
    pub fn highlighted_in(self, flat: &FlatRows) -> Option<usize> {
        self.query.get();
        self.list
            .index_tracked(flat)
            .or_else(|| (0..flat.len()).find(|&i| flat.availability(i).focusable()))
    }

    /// The highlighted value, untracked.
    #[must_use]
    pub fn highlighted(self) -> Option<String> {
        let list = self.first_list()?;
        gui::untracked(|| {
            let flat = list.flat.get();
            self.highlighted_in(&flat.0)
                .and_then(|i| flat.0.item(i).map(|n| n.value.clone()))
        })
    }

    /// Scroll `list` to the highlighted item, or to its top.
    fn reveal_in(self, list: &CommandList) {
        let flat = list.flat.get_untracked();
        let at = gui::untracked(|| self.highlighted_in(&flat.0)).and_then(|i| flat.0.reach.get(i).and_then(|n| n.row));
        match at {
            Some(r) => list.state.scroll_into_view(r),
            None => list.state.scroll_to_row(0),
        }
    }

    /// Pick `item` of `list`: its `on_select`, then the root's.
    pub fn pick(self, list: &CommandList, item: &CommandItem) {
        if item.disabled {
            return;
        }
        if let Some(f) = &list.on_select {
            f.call(item.clone());
        }
        let on_pick = self.on_pick.get_untracked();
        on_pick.call(item.clone());
    }

    /// The field's keys (the module's table): whether one was taken.
    pub fn key(self, ev: &KeyEvent) -> bool {
        if self.field.edit.with_untracked(|e| e.preedit().is_some()) {
            return false;
        }
        let Some(list) = self.first_list() else {
            return false;
        };
        let flat = list.flat.get_untracked();
        let rows = &*flat.0;
        let current = gui::untracked(|| self.highlighted_in(rows));
        let row_of = |i: usize| rows.reach.get(i).and_then(|n| n.row);
        let direction = self.list.config().direction.get();
        match step_in_list(&list.state, KEYMAP, direction, ev, rows, current, row_of) {
            RovingNav::Moved(i) => {
                self.list.highlight(rows, i, true);
                return true;
            }
            RovingNav::Stayed => return true,
            RovingNav::Ignored => {}
        }
        let m = ev.modifiers;
        if ev.is(NamedKey::Enter) && !(m.control || m.alt || m.command) {
            if let Some(item) = current.and_then(|i| rows.item(i)).cloned() {
                self.pick(&list, &item);
                return true;
            }
            return false;
        }
        false
    }

    /// A command list's state, made in the list part's setup: its rows under
    /// the search follow `sections` and `footer` (compared by pointer), laid
    /// on a virtual list of rows `row_height` tall at first; it joins the
    /// command as it is laid out ([`Command::list_node`]). A list of more
    /// than [`FILTER_AT_ONCE`] items is filtered on a thread of its own at
    /// each change of the search, the rows shown staying until the new ones
    /// come; a newer search cancels an older one.
    #[must_use]
    #[track_caller]
    pub fn list_setup(
        self,
        sections: impl Fn() -> (Arc<Vec<CommandSection>>, Arc<Vec<CommandSection>>) + 'static,
        row_height: f32,
        on_select: Option<Callback<CommandItem>>,
    ) -> CommandList {
        let state = ListState::new(row_height);
        let sections = Computed::new(move || {
            let (s, f) = sections();
            SectionsRef(s, f)
        });
        let query = self.query;
        let SectionsRef(s0, f0) = sections.get_untracked();
        let flat = Signal::new(flatten(s0, f0, &query.get_untracked()));
        // The search a filter is for: a newer one makes older ones stale.
        let generation = Arc::new(AtomicU64::new(0));
        watch(
            move || (sections.get(), query.get()),
            move |(SectionsRef(s, f), q), _| {
                let g = generation.fetch_add(1, AtomicOrdering::Relaxed).wrapping_add(1);
                let count: usize = s.iter().chain(f.iter()).map(|x| x.items().len()).sum();
                if count <= FILTER_AT_ONCE {
                    flat.set(flatten(Arc::clone(s), Arc::clone(f), q));
                    return;
                }
                let (s, f, q, at, now) = (
                    Arc::clone(s),
                    Arc::clone(f),
                    q.clone(),
                    Arc::clone(&generation),
                    Arc::clone(&generation),
                );
                // Off the UI thread, owned by the list (ADR-0026): cancelled
                // with it, and stale once a newer search starts.
                gui::spawn(
                    move |cancel| {
                        let stale = || cancel.is_cancelled() || at.load(AtomicOrdering::Relaxed) != g;
                        flatten_rows(s, f, &q, &stale)
                    },
                    move |rows| {
                        if let Some(rows) = rows
                            && g == now.load(AtomicOrdering::Relaxed)
                            && flat.is_alive()
                        {
                            flat.set(Flat(Rc::new(rows)));
                        }
                    },
                );
            },
        );
        // The list's rows follow the data: other rows have other heights,
        // and the list starts at its top again.
        let first = flat.get_untracked();
        state.splice(0, 0, u64::try_from(first.0.rows.len()).unwrap_or(u64::MAX));
        state.scroll_to_row(0);
        {
            watch(
                move || flat.get(),
                move |new, old| {
                    if new.0.rows != old.0.rows {
                        let old_len = u64::try_from(old.0.rows.len()).unwrap_or(u64::MAX);
                        state.splice(
                            0,
                            old_len.min(state.len()),
                            u64::try_from(new.0.rows.len()).unwrap_or(u64::MAX),
                        );
                        state.scroll_to_row(0);
                    }
                },
            );
        }
        // The highlighted item's row stays built: read as the list renders.
        state.active_from(move || {
            flat.with(|f| {
                self.highlighted_in(&f.0)
                    .and_then(|i| f.0.reach.get(i).and_then(|n| n.row))
            })
        });
        let list = CommandList { flat, state, on_select };
        // Once laid out, the list shows its highlighted item: what its
        // owner highlighted as it opened (a combobox, its chosen option,
        // after the first layout tells the options) is in view.
        let shown = list.clone();
        gui::after_layout(move || {
            if shown.flat.is_alive() {
                self.reveal_in(&shown);
            }
        });
        list
    }

    /// `d` as the list's node: a listbox, joining the command as it is laid
    /// out; its highlighted row stays built while scrolled away (ADR-0021:
    /// the list reads it as it renders, [`gui::list::ListState::active_from`]).
    #[must_use]
    pub fn list_node(self, d: Div, list: &CommandList) -> Div {
        d.role(gui::Role::ListBox)
            .node_ref(self.node)
            .member(&self.lists(), list.clone())
    }

    /// Whether the item valued `value` is highlighted under `rows`, tracked.
    #[must_use]
    pub fn is_highlighted(self, rows: &FlatRows, value: &str) -> bool {
        self.highlighted_in(rows)
            .and_then(|i| rows.item(i))
            .is_some_and(|n| n.value == value)
    }

    /// `d` as the row of `item` in `list`, at `set` among the options: an
    /// option to assistive technology, selected while chosen (the root's
    /// `holds`, read here), the field's active descendant while highlighted
    /// (`highlighted`, what the row's render read); the pointer highlights
    /// it and a click picks it. A disabled one does neither. `group` is the
    /// heading of its group, told after its name: the rows lie flat on a
    /// virtual list, so no group node holds its options, and a relation to
    /// a heading scrolled out of view would not be told.
    #[must_use]
    pub fn option(
        self,
        d: Div,
        list: &CommandList,
        item: &CommandItem,
        group: Option<&str>,
        highlighted: bool,
        set: Option<(usize, usize)>,
    ) -> Div {
        let holds = self.holds.with_untracked(Rc::clone);
        let d = d.role(gui::Role::ListBoxOption).aria_selected(holds(item));
        let d = match group {
            Some(g) if !g.is_empty() => d.aria_description(g),
            _ => d,
        };
        let d = match set {
            Some((at, of)) => d.aria_pos_in_set(at, of),
            None => d,
        };
        let d = if highlighted { d.node_ref(self.list.active()) } else { d };
        if item.disabled {
            return d.disabled(true);
        }
        let (list, picked, hovered) = (list.clone(), item.clone(), item.value.clone());
        let flat = list.flat;
        d.on_hover(move |inside| {
            if inside {
                let rows = flat.get_untracked();
                // Found by its place, never by a walk of a million rows.
                let at = rows
                    .0
                    .index_at(set, &hovered, false)
                    .or_else(|| rows.0.index_at(set, &hovered, true))
                    .or_else(|| rows.0.position(&hovered));
                if let Some(i) = at {
                    self.list.highlight(&*rows.0, i, false);
                }
            }
        })
        .on_click(move |_| self.pick(&list, &picked))
    }

    /// `d` as the search field: a combobox to assistive technology,
    /// expanded, controlling the list and naming the highlighted item
    /// (research §4 item 14); it takes the list's keys before its own
    /// editing, Home and End included (reka's `ListboxFilter`).
    #[must_use]
    pub fn field<K>(self, d: Node<K>) -> Node<K> {
        d.on_key_down(move |ev| self.key(ev))
            .role(gui::Role::ComboBox)
            .aria_expanded(true)
            .aria_auto_complete(gui::AutoComplete::List)
            .controls(self.node)
            .active_descendant(Some(self.list.active()))
    }
}

/// Sections compared by pointer (a computed's cutoff).
#[derive(Clone)]
struct SectionsRef(Arc<Vec<CommandSection>>, Arc<Vec<CommandSection>>);

impl PartialEq for SectionsRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) && Arc::ptr_eq(&self.1, &other.1)
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandItem, CommandSection, Row, flatten, options_of};

    #[test]
    fn a_search_hides_items_groups_and_shows_the_empty_state() {
        let sections = vec![
            CommandSection::group(Some("Fruit"), [CommandItem::new("Apple"), CommandItem::new("Banana")]),
            CommandSection::Separator,
            CommandSection::group(None, [CommandItem::new("Café")]),
            CommandSection::empty("No results."),
        ];
        let sections = std::sync::Arc::new(sections);
        let none_footer = std::sync::Arc::new(Vec::new());
        let all = flatten(sections.clone(), none_footer.clone(), "");
        assert_eq!(all.0.reach.len(), 3);
        assert!(!all.0.rows.iter().any(|r| matches!(r, Row::Empty { .. })));
        let cafe = flatten(sections.clone(), none_footer.clone(), "cafe");
        assert_eq!(cafe.0.reach.len(), 1, "accents aside");
        assert!(
            !cafe.0.rows.iter().any(|r| matches!(r, Row::Heading { .. })),
            "a group with nothing left hides"
        );
        let none = flatten(sections, none_footer, "zzz");
        assert!(none.0.rows.iter().any(|r| matches!(r, Row::Empty { .. })));
        assert_eq!(options_of(&all.0.rows).iter().flatten().count(), 3);
        let bare = std::sync::Arc::new(vec![CommandSection::empty("No results.")]);
        let bare = flatten(bare, std::sync::Arc::new(Vec::new()), "");
        assert!(
            !bare.0.rows.iter().any(|r| matches!(r, Row::Empty { .. })),
            "no items and no search: no empty state (`CommandEmpty.vue:15`)"
        );
    }
}
