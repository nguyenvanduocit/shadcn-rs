//! Command (`ui/command`): a search box over a list of items, filtered as
//! you type, highlighted by the pointer and the arrow keys, picked with a
//! click or Enter. Combobox's list is a command list too.
//!
//! As in shadcn-vue's `Command.vue`, every item is known by its value: an
//! item hides when its value does not contain the search (case-insensitive),
//! a group hides when all its items do, and the empty state shows when a
//! search hides them all.
//!
//! The list is data ([`CommandSection`]s of [`CommandItem`]s) laid out on a
//! virtual list (ADR-0014): only the rows in view and a margin around them
//! are built, so a palette of a million items opens and filters as one of
//! ten does. The keys walk the data, not what is built: the highlight is an
//! index into the items the search leaves, and the row it lands on is
//! scrolled into view (built if it was not).
//!
//! The parts nest as in Vue: [`command`] keeps the search (its own, or the
//! caller's, `search`) and the highlight (and the value picked last,
//! `model`); its [`command_input`] and [`command_list`] read them.

use std::cell::Cell;
use std::rc::Rc;

use gui::list::{ListState, virtual_list_in};
use gui::prelude::*;
use gui::{
    Bind, Callback, Computed, Div, ElementKey, Icon, KeyEvent, Members, Model, NamedKey, Signal, Slot, Text, TextField,
    component, div, icon, key_of, provide, text, text_input, use_context, use_model, watch,
};

use crate::dropdown_menu::{menu_nav_key, menu_step};
use crate::part::Part;
use crate::theme::theme;

/// An item of a command list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandItem {
    /// What the search matches and what picking it hands over.
    pub value: String,
    /// Shown dimmed, skipped by the keys, not picked.
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

    /// An item shown dimmed that cannot be picked.
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
    /// A group (`p-1`) under an optional heading; hidden when the search
    /// hides all its items.
    Group {
        heading: Option<String>,
        items: Vec<CommandItem>,
    },
    /// Items straight in the list, no group around them (reka's `ListboxContent`).
    Items(Vec<CommandItem>),
    /// `bg-border -mx-1 h-px`, always shown.
    Separator,
    /// `py-6 text-center text-sm`, shown when no item shows.
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

    fn items(&self) -> &[CommandItem] {
        match self {
            Self::Group { items, .. } | Self::Items(items) => items,
            Self::Separator | Self::Empty(_) => &[],
        }
    }
}

/// Whether an item valued `value` shows under `search`.
#[must_use]
pub fn command_matches(value: &str, search: &str) -> bool {
    search.is_empty() || value.to_lowercase().contains(&search.to_lowercase())
}

/// What is highlighted, and the search it was highlighted under: once the
/// search changes, the first item is highlighted instead (reka highlights the
/// first item on every search).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Highlight {
    /// `None`: the first item.
    value: Option<String>,
    search: String,
}

/// A row of the list as laid out: one per heading, item, separator and
/// empty state the search leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading {
        section: usize,
    },
    /// `first`/`last`: the first and last rows of its group (its padding).
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

/// An item the keys reach: where it is in the data and the row it is on
/// (`None` in a footer, which is not virtual).
#[derive(Clone, Debug)]
struct Nav {
    item: CommandItem,
    row: Option<u64>,
}

/// What the rows of a list are named (`command-item`, `combobox-item`, …).
#[derive(Clone, Copy)]
pub(crate) struct Names {
    pub list: &'static str,
    pub group: &'static str,
    pub heading: &'static str,
    pub item: &'static str,
    pub separator: &'static str,
    pub empty: &'static str,
}

const COMMAND_NAMES: Names = Names {
    list: "command-list",
    group: "command-group",
    heading: "command-group-heading",
    item: "command-item",
    separator: "command-separator",
    empty: "command-empty",
};

/// What a list shows under the current search: its rows, its footer's,
/// their keys, and the items the keys reach. Compared by pointer, so a
/// million rows are never compared.
#[derive(Clone)]
struct Flat(Rc<FlatRows>);

struct FlatRows {
    rows: Vec<Row>,
    tail: Vec<Row>,
    keys: Vec<ElementKey>,
    nav: Vec<Nav>,
}

impl PartialEq for Flat {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Sections compared by pointer (a computed's cutoff).
#[derive(Clone)]
struct SectionsRef(Rc<Vec<CommandSection>>, Rc<Vec<CommandSection>>);

impl PartialEq for SectionsRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) && Rc::ptr_eq(&self.1, &other.1)
    }
}

/// A list inside a command, as its keys reach it.
#[derive(Clone)]
struct ListHandle {
    flat: Computed<Flat>,
    list: ListState,
    on_select: Option<Callback<CommandItem>>,
}

/// What the parts of a command read (and a combobox, whose list is a
/// command's): the search, the highlight, its lists as last laid out, and
/// what picking an item does at the root.
#[derive(Clone)]
pub(crate) struct CommandContext {
    /// The search box (`filterState.search`).
    pub(crate) search: TextField,
    /// The search's text, cut off from caret moves.
    query: Computed<String>,
    highlight: Signal<Highlight>,
    lists: Members<ListHandle>,
    /// Bring the highlighted item into view as the list mounts (a combobox
    /// opening on its chosen value).
    reveal: Rc<Cell<bool>>,
    /// What the root does with an item picked.
    on_pick: Rc<dyn Fn(&CommandItem)>,
}

impl CommandContext {
    /// A command's state, made in its root's setup, filtered by what is
    /// typed in `search`; `on_pick` runs at the root.
    #[track_caller]
    pub(crate) fn new(search: TextField, on_pick: impl Fn(&CommandItem) + 'static) -> Self {
        Self {
            search,
            query: Computed::new(move || search.edit.with(|e| e.text.clone())),
            highlight: Signal::labeled("command highlight", Highlight::default()),
            lists: Members::new(),
            reveal: Rc::new(Cell::new(false)),
            on_pick: Rc::new(on_pick),
        }
    }

    /// The nearest command's, from a part's setup.
    pub(crate) fn find() -> Option<Self> {
        use_context::<Self>()
    }

    fn list(&self) -> Option<ListHandle> {
        self.lists.with(|l| l.first().cloned())
    }

    /// The highlighted item's index among `nav`, tracked.
    fn highlighted_in(&self, nav: &[Nav]) -> Option<usize> {
        let query = self.query.get();
        self.highlight.with(|h| match &h.value {
            Some(v) if h.search == query => nav.iter().position(|n| n.item.value == *v),
            _ => (!nav.is_empty()).then_some(0),
        })
    }

    /// The highlighted value, untracked.
    #[cfg(test)]
    fn highlighted(&self) -> Option<String> {
        let list = self.list()?;
        gui::untracked(|| {
            let flat = list.flat.get();
            self.highlighted_in(&flat.0.nav)
                .and_then(|i| flat.0.nav.get(i).map(|n| n.item.value.clone()))
        })
    }

    /// Highlight `value` under the current search.
    fn highlight(&self, value: Option<String>) {
        let next = Highlight {
            value,
            search: self.query.get_untracked(),
        };
        if self.highlight.get_untracked() != next {
            self.highlight.set(next);
        }
    }

    /// Highlight `value`, or the first item when it is `None`, and show it
    /// (a combobox opening on its chosen value).
    pub(crate) fn highlight_and_reveal(&self, value: Option<String>) {
        self.highlight(value);
        match self.list() {
            Some(list) => reveal(self, &list),
            None => self.reveal.set(true),
        }
    }

    fn pick(&self, list: &ListHandle, item: &CommandItem) {
        if let Some(f) = &list.on_select {
            f.call(item.clone());
        }
        (self.on_pick)(item);
        self.search.set_text("");
    }

    /// The list's keys: arrows and Home/End move the highlight over the
    /// items the search leaves (built or not), scrolling the one reached
    /// into view; Enter picks. [`command`] takes them itself; a search field
    /// outside it (a combobox's list) hands them over.
    pub(crate) fn key(&self, ev: &KeyEvent) -> bool {
        let Some(list) = self.list() else {
            return false;
        };
        let flat = list.flat.get_untracked();
        let nav = &flat.0.nav;
        let current = gui::untracked(|| self.highlighted_in(nav));
        if let Some(step) = menu_nav_key(ev) {
            if let Some(n) = menu_step(nav.len(), current, step).and_then(|i| nav.get(i)) {
                self.highlight(Some(n.item.value.clone()));
                if let Some(row) = n.row {
                    list.list.scroll_into_view(row);
                }
            }
            return true;
        }
        if ev.is(NamedKey::Enter) {
            if let Some(item) = current.and_then(|i| nav.get(i)).map(|n| n.item.clone()) {
                self.pick(&list, &item);
            }
            return true;
        }
        false
    }
}

/// Scroll `list` to the highlighted item, or to its top.
fn reveal(cmd: &CommandContext, list: &ListHandle) {
    let flat = list.flat.get_untracked();
    let at = gui::untracked(|| cmd.highlighted_in(&flat.0.nav)).and_then(|i| flat.0.nav.get(i).and_then(|n| n.row));
    match at {
        Some(r) => list.list.scroll_into_view(r),
        None => list.list.scroll_to_row(0),
    }
}

/// What [`command`] is given: the value picked last (reka's `modelValue`),
/// and its search (`filterState.search`, `v-model:search`).
#[derive(Clone, PartialEq, Default)]
pub struct CommandProps {
    value: Bind<String>,
    search: Bind<String>,
}

impl Part<CommandProps> {
    /// The value picked, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// Call `f` with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(String) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// The search, kept by the caller (`v-model:search`): what its
    /// [`command_input`] shows and what filters the list. A field outside
    /// the command that binds the same model filters it too (a listbox's
    /// filter: a tags input's [`text`](Part::text)).
    #[must_use]
    pub fn search(self, search: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.search = p.search.clone().with_model(search))
    }

    /// The search it starts with when no `search` model is given.
    #[must_use]
    pub fn default_search(self, search: impl Into<String>) -> Self {
        self.with_props(|p| p.search = p.search.clone().with_default(search.into()))
    }

    /// Call `f` with the search after each change (`@update:search`).
    #[must_use]
    pub fn on_search_change(self, f: impl Fn(String) + 'static) -> Self {
        self.with_props(|p| p.search = p.search.clone().on_change(f))
    }
}

/// The root: `bg-popover text-popover-foreground flex h-full w-full flex-col
/// overflow-hidden rounded-md`; it keeps the search and the highlight and
/// takes the list's keys. Give it a [`command_input`] and a [`command_list`].
#[must_use]
pub fn command() -> Part<CommandProps> {
    let t = theme();
    let root = div()
        .named("command")
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .flex()
        .h_full()
        .w_full()
        .flex_col()
        .overflow_hidden()
        .rounded_md();
    Part::new(CommandProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p| &p.props.value);
            let typed = use_model(props, |p| &p.props.search);
            let search = TextField::new(typed.get_untracked());
            crate::part::bind_field(search, typed);
            let cmd = CommandContext::new(search, move |item: &CommandItem| {
                if !item.null {
                    value.set(item.value.clone());
                }
            });
            provide(cmd.clone());
            move || {
                let keys = cmd.clone();
                props
                    .with(|p| p.slot.0.clone())
                    .on_key_down(move |ev| keys.key(ev))
                    .into_element()
            }
        })
    })
}

/// The search box: `flex h-9 items-center gap-2 border-b px-3`, a faded
/// search icon and the input (`h-10 py-3 text-sm`, no outline), focused as
/// it mounts (`auto-focus`).
#[must_use]
pub fn command_input(placeholder: &str) -> Part<String> {
    let t = theme();
    let root = div()
        .named("command-input-wrapper")
        .flex()
        .h(9.0)
        .items_center()
        .gap(2.0)
        .border_b()
        .border_color(t.border)
        .px(3.0)
        .child(icon(Icon::Search).shrink_0().opacity(0.5));
    Part::new(placeholder.to_owned(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let cmd = CommandContext::find();
            if let Some(c) = &cmd {
                let focus = c.search.focus;
                gui::on_mounted(move || focus.focus());
            }
            move || {
                let (placeholder, root) = props.with(|p| (p.props.clone(), p.slot.0.clone()));
                let Some(cmd) = &cmd else {
                    return root.into_element();
                };
                let t = theme();
                root.child(
                    text_input(&cmd.search)
                        .named("command-input")
                        .placeholder(placeholder)
                        .placeholder_color(t.muted_foreground)
                        .selection_color(t.primary.opacity(0.35))
                        .flex()
                        .h(10.0)
                        .w_full()
                        .rounded_md()
                        .py(3.0)
                        .text_sm()
                        .disabled_style(|s| s.cursor_not_allowed().opacity(0.5)),
                )
                .into_element()
            }
        })
    })
}

/// What adds an item's icon, label and shortcut to its row.
type RowFn = Rc<dyn Fn(&CommandItem, Div) -> Div>;

/// A command's list (`max-h-[300px] overflow-y-auto`): `sections` filtered
/// by the search, `row` adding each item's icon, label and shortcut to its
/// row (`|item, row| row.child(…)`). Icons take `text-muted-foreground` from
/// the caller, as shadcn's `[&_svg]` rule gives them. Only the rows in view
/// are built. Hold the sections in an `Rc` the caller keeps, so a list of a
/// million items is filtered again only when the search changes.
pub fn command_list(
    sections: impl Into<Rc<Vec<CommandSection>>>,
    row: impl Fn(&CommandItem, Div) -> Div + 'static,
) -> CommandList {
    CommandList {
        sections: sections.into(),
        footer: Rc::new(Vec::new()),
        on_select: None,
        row: Slot(Rc::new(row)),
        max_h: Some(300.0),
        names: COMMAND_NAMES,
    }
}

/// A command list being built; it is an element.
#[must_use]
#[derive(Clone)]
pub struct CommandList {
    sections: Rc<Vec<CommandSection>>,
    footer: Rc<Vec<CommandSection>>,
    on_select: Option<Callback<CommandItem>>,
    row: Slot<RowFn>,
    max_h: Option<f32>,
    names: Names,
}

impl PartialEq for CommandList {
    fn eq(&self, _: &Self) -> bool {
        // The row closure is the parent's, made anew each render.
        false
    }
}

impl CommandList {
    /// Run `f` with the item picked (`@select`).
    pub fn on_select(mut self, f: impl Fn(&CommandItem) + 'static) -> Self {
        self.on_select = Some(Callback::new(move |item: CommandItem| f(&item)));
        self
    }

    /// At most `px` tall, scrolling past it (`max-h-[…]`).
    pub fn max_h_px(mut self, px: f32) -> Self {
        self.max_h = Some(px);
        self
    }

    /// As tall as its rows, never scrolling (a list with no viewport).
    pub fn unbounded(mut self) -> Self {
        self.max_h = None;
        self
    }

    /// Sections below the scrolling rows, always in view (a "Create …"
    /// group under a combobox's viewport); the keys reach their items after
    /// the list's.
    pub fn footer(mut self, sections: impl Into<Rc<Vec<CommandSection>>>) -> Self {
        self.footer = sections.into();
        self
    }

    pub(crate) fn names(mut self, names: Names) -> Self {
        self.names = names;
        self
    }
}

impl IntoElement for CommandList {
    fn into_element(self) -> gui::Element {
        component(self, |cx| {
            let props = cx.props();
            let Some(cmd) = CommandContext::find() else {
                let empty: Box<dyn FnMut() -> gui::Element> = Box::new(|| gui::Element::Empty);
                return empty;
            };
            let list = ListState::new(32.0);
            let sections =
                Computed::new(move || props.with(|p| SectionsRef(Rc::clone(&p.sections), Rc::clone(&p.footer))));
            let query = cmd.query;
            let flat = Computed::new(move || {
                let SectionsRef(sections, footer) = sections.get();
                let search = query.get();
                let mut nav = Vec::new();
                let rows = flatten(&sections, &search, &mut nav, true);
                let tail = flatten(&footer, &search, &mut nav, false);
                let keys = rows.iter().map(|r| row_key(&sections, r)).collect();
                Flat(Rc::new(FlatRows { rows, tail, keys, nav }))
            });
            // The list's rows follow the data: other rows have other heights,
            // and the list starts at its top again.
            let first = flat.get_untracked();
            list.splice(0, 0, u64::try_from(first.0.keys.len()).unwrap_or(u64::MAX));
            // Rows inserted before the anchor push it down: start at the top.
            list.scroll_to_row(0);
            {
                let list = list.clone();
                watch(
                    move || flat.get(),
                    move |new, old| {
                        if new.0.keys != old.0.keys {
                            let old_len = u64::try_from(old.0.keys.len()).unwrap_or(u64::MAX);
                            list.splice(
                                0,
                                old_len.min(list.len()),
                                u64::try_from(new.0.keys.len()).unwrap_or(u64::MAX),
                            );
                            list.scroll_to_row(0);
                        }
                    },
                );
            }
            let handle = move || ListHandle {
                flat,
                list: list.clone(),
                on_select: props.with_untracked(|p| p.on_select.clone()),
            };
            if cmd.reveal.replace(false) {
                reveal(&cmd, &handle());
            }
            Box::new(move || {
                let h = handle();
                let flat = flat.get();
                let FlatRows { rows, tail, keys, nav } = &*flat.0;
                let (sections, footer, row, max_h, names) = props.with(|p| {
                    (
                        Rc::clone(&p.sections),
                        Rc::clone(&p.footer),
                        Rc::clone(&p.row.0),
                        p.max_h,
                        p.names,
                    )
                });
                let list = &h.list;
                list.fit_rows(Some(max_h.unwrap_or(f32::INFINITY)));
                let at = cmd.highlighted_in(nav);
                let on = at.and_then(|i| nav.get(i).map(|n| n.item.value.clone()));
                // The highlighted option stays built while scrolled away (ADR-0021).
                list.set_active(at.and_then(|i| nav.get(i).and_then(|n| n.row)));
                // Each option's place among the options (headings and separators are not).
                let options = options_of(rows);
                let ctx = Rows {
                    cmd: &cmd,
                    list: &h,
                    on: on.as_deref(),
                    row: &*row,
                    names,
                };
                let n = u64::try_from(rows.len()).unwrap_or(u64::MAX);
                let body = virtual_list_in(
                    list,
                    n,
                    |i| {
                        keys.get(usize::try_from(i).unwrap_or(usize::MAX))
                            .copied()
                            .unwrap_or_default()
                    },
                    |i| {
                        let i = usize::try_from(i).unwrap_or(usize::MAX);
                        rows.get(i).map_or(gui::Element::Empty, |r| {
                            ctx.build(&sections, r, options.get(i).copied().flatten())
                                .into_element()
                        })
                    },
                    |viewport| viewport.overflow_x_hidden(),
                )
                .named(names.list)
                .role(gui::Role::ListBox)
                .w_full()
                .member(&cmd.lists, h.clone());
                if tail.is_empty() {
                    return body.into_element();
                }
                let tail_options = options_of(tail);
                let tail_rows = tail
                    .iter()
                    .enumerate()
                    .map(|(i, r)| ctx.build(&footer, r, tail_options.get(i).copied().flatten()));
                div().flex_col().child(body).children(tail_rows).into_element()
            })
        })
        .named("command-list")
        .into_element()
    }
}

/// What building a row reads.
struct Rows<'a> {
    cmd: &'a CommandContext,
    list: &'a ListHandle,
    on: Option<&'a str>,
    row: &'a dyn Fn(&CommandItem, Div) -> Div,
    names: Names,
}

impl Rows<'_> {
    /// One row: a group's part (`text-foreground overflow-hidden p-1` spread
    /// over its rows), a separator or the empty state.
    fn build(&self, sections: &[CommandSection], r: &Row, set: Option<(usize, usize)>) -> Div {
        let t = theme();
        let names = self.names;
        let grouped = |section: usize| matches!(sections.get(section), Some(CommandSection::Group { .. }));
        // A group's part is no row of the listbox: its options are.
        let group_part = |section: usize, first: bool, last: bool| {
            div()
                .named(names.group)
                .role(gui::Role::GenericContainer)
                .text_color(t.foreground)
                .overflow_hidden()
                .px(1.0)
                .when(first, |d| d.pt(1.0))
                .when(last, |d| d.pb(1.0))
                .when(!grouped(section), |d| d.px(0.0))
        };
        match *r {
            Row::Heading { section } => {
                let heading = match sections.get(section) {
                    Some(CommandSection::Group { heading: Some(h), .. }) => h.clone(),
                    _ => String::new(),
                };
                group_part(section, true, false).child(
                    div()
                        .named(names.heading)
                        .px(2.0)
                        .py(1.5)
                        .text_xs()
                        .font_medium()
                        .text_color(t.muted_foreground)
                        .child(heading),
                )
            }
            Row::Item {
                section,
                item,
                first,
                last,
            } => {
                let Some(it) = sections.get(section).and_then(|s| s.items().get(item)) else {
                    return div();
                };
                group_part(section, first, last).child((self.row)(it, self.item_row(it, set)))
            }
            Row::Separator { .. } => command_separator().named(names.separator).role(gui::Role::Splitter),
            Row::Empty { section } => {
                let message = match sections.get(section) {
                    Some(CommandSection::Empty(m)) => m.as_str(),
                    _ => "",
                };
                command_empty_box(message)
                    .named(names.empty)
                    .role(gui::Role::GenericContainer)
            }
        }
    }

    /// An item's row: highlighted, hovered and clicked as the list's; an
    /// option to a screen reader, at its place `set` among the options.
    fn item_row(&self, item: &CommandItem, set: Option<(usize, usize)>) -> Div {
        let on = self.on == Some(item.value.as_str());
        let d = command_item_row()
            .named(self.names.item)
            .role(gui::Role::ListBoxOption)
            .aria_selected(on);
        let d = match set {
            Some((at, of)) => d.aria_pos_in_set(at, of),
            None => d,
        };
        if item.disabled {
            return d.disabled(true).opacity(0.5);
        }
        let t = theme();
        let (hover, click, list) = (self.cmd.clone(), self.cmd.clone(), self.list.clone());
        let (hovered, picked) = (item.value.clone(), item.clone());
        d.when(on, |d| d.bg(t.accent).text_color(t.accent_foreground))
            .on_hover(move |inside| {
                if inside {
                    hover.highlight(Some(hovered.clone()));
                }
            })
            .on_click(move |_| click.pick(&list, &picked))
    }
}

/// The rows `sections` show under `search`, and the items the keys reach.
fn flatten(sections: &[CommandSection], search: &str, nav: &mut Vec<Nav>, virtual_rows: bool) -> Vec<Row> {
    let any = sections
        .iter()
        .any(|s| s.items().iter().any(|i| command_matches(&i.value, search)));
    let mut rows = Vec::new();
    for (section, s) in sections.iter().enumerate() {
        match s {
            CommandSection::Separator => rows.push(Row::Separator { section }),
            CommandSection::Empty(_) => {
                if !any {
                    rows.push(Row::Empty { section });
                }
            }
            CommandSection::Group { .. } | CommandSection::Items(_) => {
                let items = s.items();
                let heading = match s {
                    CommandSection::Group { heading, .. } => heading.as_ref(),
                    _ => None,
                };
                let shown: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| command_matches(&i.value, search))
                    .map(|(k, _)| k)
                    .collect();
                if shown.is_empty() {
                    continue;
                }
                let grouped = matches!(s, CommandSection::Group { .. });
                if grouped && heading.is_some() {
                    rows.push(Row::Heading { section });
                }
                let last_k = shown.len().saturating_sub(1);
                for (k, &item) in shown.iter().enumerate() {
                    let at = u64::try_from(rows.len()).unwrap_or(u64::MAX);
                    if let Some(i) = items.get(item).filter(|i| !i.disabled) {
                        nav.push(Nav {
                            item: i.clone(),
                            row: virtual_rows.then_some(at),
                        });
                    }
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
    rows
}

/// Each row's place among the item rows, from 1, and how many there are.
fn options_of(rows: &[Row]) -> Vec<Option<(usize, usize)>> {
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

pub(crate) fn command_empty_box(message: &str) -> Div {
    div().py(6.0).text_center().text_sm().child(message.to_owned())
}

/// The look of an item: `relative flex cursor-default items-center gap-2
/// rounded-sm px-2 py-1.5 text-sm select-none`.
pub(crate) fn command_item_row() -> Div {
    div()
        .relative()
        .flex()
        .cursor_default()
        .items_center()
        .gap(2.0)
        .rounded_sm()
        .px(2.0)
        .py(1.5)
        .text_sm()
}

/// `bg-border -mx-1 h-px`.
#[must_use]
pub fn command_separator() -> Div {
    let t = theme();
    div().named("command-separator").bg(t.border).mx(-1.0).h_px(1.0)
}

/// `text-muted-foreground ml-auto text-xs tracking-widest`.
#[must_use]
pub fn command_shortcut(keys: &str) -> Text {
    let t = theme();
    text(keys)
        .named("command-shortcut")
        .text_color(t.muted_foreground)
        .ml_auto()
        .text_xs()
        .tracking_widest()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use gui::{Modifiers, Scenario, Signal, text};

    const ITEMS: usize = 10_000;

    thread_local! {
        /// The context the probe found.
        static FOUND: RefCell<Option<CommandContext>> = const { RefCell::new(None) };
    }

    /// A part inside the command that hands its context to the test.
    fn probe() -> Part<()> {
        Part::new((), div(), |p| {
            component(p, |cx| {
                FOUND.set(CommandContext::find());
                let props = cx.props();
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        })
    }

    fn palette(search: Signal<String>, picked: Signal<String>) -> gui::Element {
        let sections = vec![
            CommandSection::group(
                Some("Letters"),
                [CommandItem::new("alpha"), CommandItem::disabled("beta")],
            ),
            CommandSection::Separator,
            CommandSection::group(
                Some("Numbers"),
                (0..ITEMS).map(|i| CommandItem::new(format!("number {i}"))),
            ),
            CommandSection::empty("No results found."),
        ];
        command()
            .search(search)
            .w_px(400.0)
            .child(probe())
            .child(
                command_list(sections, |item, row| row.child(text(item.value.clone())))
                    .on_select(move |item| picked.set(item.value.clone())),
            )
            .into_element()
    }

    /// The items' rows built.
    fn built(s: &Scenario) -> Vec<String> {
        s.last()
            .nodes
            .iter()
            .filter_map(|n| match &n.content {
                gui::layout::Content::Text { content, .. } => Some(content.to_string()),
                _ => None,
            })
            .filter(|t| t == "alpha" || t == "beta" || t.starts_with("number "))
            .collect()
    }

    fn press(s: &mut Scenario, cmd: &CommandContext, key: NamedKey) {
        let ev = KeyEvent {
            key: gui::Key::Named(key),
            text: None,
            modifiers: Modifiers::default(),
        };
        s.act("key", || {
            cmd.key(&ev);
        });
        s.settle().unwrap();
    }

    fn mount() -> (Scenario, CommandContext, Signal<String>, Signal<String>) {
        let search = Signal::new(String::new());
        let picked = Signal::new(String::new());
        let mut s = Scenario::measuring(move || palette(search, picked), 400, 500, 1.0).unwrap();
        s.settle().unwrap();
        let cmd = FOUND
            .with_borrow(Clone::clone)
            .expect("the command provides its context");
        (s, cmd, search, picked)
    }

    #[test]
    fn the_keys_walk_the_data_and_bring_rows_never_built_into_view() {
        let (mut s, cmd, _, picked) = mount();
        // Ten thousand items, a few dozen rows built.
        assert!(built(&s).len() < 60, "{} rows built", built(&s).len());
        assert_eq!(cmd.highlighted().as_deref(), Some("alpha"));
        // Down skips the disabled item.
        press(&mut s, &cmd, NamedKey::ArrowDown);
        assert_eq!(cmd.highlighted().as_deref(), Some("number 0"));
        // End reaches the last item, far outside what was built.
        assert!(!built(&s).contains(&format!("number {}", ITEMS - 1)));
        press(&mut s, &cmd, NamedKey::End);
        assert_eq!(cmd.highlighted(), Some(format!("number {}", ITEMS - 1)));
        assert!(built(&s).contains(&format!("number {}", ITEMS - 1)));
        assert!(built(&s).len() < 60);
        press(&mut s, &cmd, NamedKey::ArrowUp);
        assert_eq!(cmd.highlighted(), Some(format!("number {}", ITEMS - 2)));
        press(&mut s, &cmd, NamedKey::Home);
        assert_eq!(cmd.highlighted().as_deref(), Some("alpha"));
        assert!(built(&s).contains(&String::from("alpha")));
        press(&mut s, &cmd, NamedKey::End);
        press(&mut s, &cmd, NamedKey::Enter);
        assert_eq!(picked.get(), format!("number {}", ITEMS - 1));
    }

    #[test]
    fn a_search_filters_the_data_and_highlights_its_first_item() {
        let (mut s, cmd, search, _) = mount();
        s.act("search", || search.set("number 99".to_owned()));
        s.settle().unwrap();
        assert_eq!(cmd.highlighted().as_deref(), Some("number 99"));
        // 99, 990..=999, 9900..=9999: 111 items, the first few built.
        press(&mut s, &cmd, NamedKey::End);
        assert_eq!(cmd.highlighted().as_deref(), Some("number 9999"));
        assert!(built(&s).contains(&String::from("number 9999")));
        s.act("nothing", || search.set("zzz".to_owned()));
        s.settle().unwrap();
        assert_eq!(cmd.highlighted(), None);
        assert!(s.last().nodes.iter().any(|n| matches!(&n.content,
            gui::layout::Content::Text { content, .. } if content.as_str() == "No results found.")));
    }
}
