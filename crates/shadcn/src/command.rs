//! Command (`ui/command`): a search box over a list of items, filtered as
//! you type, highlighted by the pointer and the arrow keys, picked with a
//! click or Enter. Combobox's list is a command list too.
//!
//! The behaviour is `primitives::command`'s (the filter, the highlight, the
//! keys, the pick, what assistive technology is told); this module draws it.
//! The list is data ([`CommandSection`]s of [`CommandItem`]s) laid out on a
//! virtual list (ADR-0014): only the rows in view and a margin around them
//! are built, so a palette of a million items opens and filters as one of
//! ten does.
//!
//! The parts nest as in Vue: [`command`] keeps the search (its own, or the
//! caller's, `search`) and the highlight (and the value picked last,
//! `model`); its [`command_input`] and [`command_list`] read them.

use std::rc::Rc;
use std::sync::Arc;

use gui::list::virtual_list_in;
use gui::prelude::*;
use gui::{
    Bind, Callback, Div, Icon, Model, Slot, Text, TextField, component, div, icon, provide, text, text_input,
    use_context, use_model,
};

use primitives::command::{Command, CommandList as ListModel, FlatRows, Row, options_of};
pub use primitives::command::{CommandItem, CommandSection};

use crate::focus::StateLook;
use crate::part::Part;
use crate::theme::theme;

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

/// What a list joining a command does as it is set up (a combobox reads its
/// options through it).
pub(crate) type OnList = gui::Callback<ListModel>;

/// What the parts of a command read (and a combobox's, whose list is a
/// command's): the command and its search field.
#[derive(Clone)]
pub(crate) struct CommandContext {
    pub(crate) cmd: Command,
    /// The search box (`filterState.search`).
    pub(crate) search: TextField,
    pub(crate) on_list: Option<OnList>,
}

impl CommandContext {
    /// The nearest command's, from a part's setup.
    pub(crate) fn find() -> Option<Self> {
        use_context::<Self>()
    }

    /// The highlighted value, untracked.
    #[cfg(test)]
    fn highlighted(&self) -> Option<String> {
        self.cmd.highlighted()
    }
}

/// What [`command`] is given: the value picked last (reka's `modelValue`),
/// and its search (`filterState.search`, `v-model:search`).
#[derive(Clone, PartialEq)]
pub struct CommandProps {
    value: Bind<String>,
    search: Bind<String>,
}

impl Default for CommandProps {
    /// No value of its own: shadcn-vue's `Command` passes `modelValue`
    /// `''` unless the caller binds one, so without a [`model`](Part::model)
    /// a pick is an action (heard by `on_change`) and nothing is ever read
    /// as selected.
    fn default() -> Self {
        Self {
            value: Bind::value(String::new()),
            search: Bind::default(),
        }
    }
}

impl Part<CommandProps> {
    /// The value picked, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// Call `f` with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<String>>) -> Self {
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
    pub fn on_search_change(self, f: impl Into<gui::Callback<String>>) -> Self {
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
        .background(t.popover)
        .text_color(t.popover_foreground)
        .height(pct(100.0))
        .width(pct(100.0))
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .overflow(Overflow::Hidden)
        .radius(8.0);
    Part::new(CommandProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p| &p.props.value);
            let typed = use_model(props, |p| &p.props.search);
            let search = TextField::new(typed.get_untracked());
            crate::part::bind_field(search, typed);
            let cmd = Command::new(
                search,
                move || search.text(),
                Command::listbox(),
                move |item: &CommandItem| !item.null && value.with(|v| *v == item.value),
                move |item: CommandItem| {
                    if !item.null {
                        value.set(item.value);
                    }
                    search.set_text("");
                },
            );
            provide(CommandContext {
                cmd,
                search,
                on_list: None,
            });
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .on_key_down(move |ev| cmd.key(ev))
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
        .display(Display::Flex)
        .height(36.0)
        .align_items(Align::Center)
        .gap(8.0)
        .border_bottom(1.0)
        .border_color(t.border)
        .padding_x(12.0)
        .child(icon(Icon::Search).flex_shrink(0.0).opacity(0.5));
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
                let input = text_input(&cmd.search)
                    .named("command-input")
                    .placeholder(placeholder)
                    .placeholder_color(t.muted_foreground)
                    .selection_color(t.primary.opacity(0.35))
                    .display(Display::Flex)
                    .height(40.0)
                    .width(pct(100.0))
                    .radius(8.0)
                    .padding_y(12.0)
                    .text_size(14.0)
                    .line_height(20.0)
                    .with_interaction(|d, i| d.cursor_disabled(i).dim_disabled(i, 0.5));
                root.child(cmd.cmd.field(input)).into_element()
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
/// million items is filtered again only when the search changes, and off
/// the UI thread (`primitives::command::FILTER_AT_ONCE`).
pub fn command_list(
    sections: impl Into<Arc<Vec<CommandSection>>>,
    row: impl Fn(&CommandItem, Div) -> Div + 'static,
) -> CommandList {
    CommandList {
        sections: sections.into(),
        footer: Arc::new(Vec::new()),
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
    sections: Arc<Vec<CommandSection>>,
    footer: Arc<Vec<CommandSection>>,
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
    pub fn on_select(mut self, f: impl Into<Callback<CommandItem>>) -> Self {
        self.on_select = Some(f.into());
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
    pub fn footer(mut self, sections: impl Into<Arc<Vec<CommandSection>>>) -> Self {
        self.footer = sections.into();
        self
    }

    pub(crate) fn names(mut self, names: Names) -> Self {
        self.names = names;
        self
    }
}

/// An item's row height before it is measured (`py-1.5` around a 20px line).
const ROW: f32 = 32.0;

impl IntoElement for CommandList {
    fn into_element(self) -> gui::Element {
        component(self, |cx| {
            let props = cx.props();
            let Some(ctx) = CommandContext::find() else {
                let empty: Box<dyn Fn() -> gui::Element> = Box::new(|| gui::Element::Empty);
                return empty;
            };
            let cmd = ctx.cmd;
            let model = cmd.list_setup(
                move || props.with(|p| (Arc::clone(&p.sections), Arc::clone(&p.footer))),
                ROW,
                // A handle: the `on_select` passed last, when it is picked.
                Some(cx.event_opt(|p| p.on_select.as_ref())),
            );
            if let Some(f) = &ctx.on_list {
                f.call(model.clone());
            }
            Box::new(move || {
                let flat = model.flat.get();
                let data = &*flat.0;
                let FlatRows { rows, tail, .. } = data;
                let (sections, footer, row, max_h, names) = props.with(|p| {
                    (
                        Arc::clone(&p.sections),
                        Arc::clone(&p.footer),
                        Rc::clone(&p.row.0),
                        p.max_h,
                        p.names,
                    )
                });
                let state = &model.state;
                state.fit_rows(Some(max_h.unwrap_or(f32::INFINITY)));
                let on = cmd
                    .highlighted_in(data)
                    .and_then(|i| data.item(i).map(|n| n.value.clone()));
                // Each option's place among the options (headings and separators are not).
                let options = options_of(rows);
                let look = Rows {
                    cmd,
                    list: &model,
                    on: on.as_deref(),
                    row: &*row,
                    names,
                };
                let n = u64::try_from(rows.len()).unwrap_or(u64::MAX);
                let body = virtual_list_in(
                    state,
                    n,
                    |i| data.key(usize::try_from(i).unwrap_or(usize::MAX)),
                    |i| {
                        let i = usize::try_from(i).unwrap_or(usize::MAX);
                        rows.get(i).map_or(gui::Element::Empty, |r| {
                            look.build(&sections, r, options.get(i).copied().flatten())
                                .into_element()
                        })
                    },
                    |viewport| viewport.overflow_x(Overflow::Hidden),
                )
                .named(names.list)
                .width(pct(100.0));
                let body = cmd.list_node(body, &model);
                if tail.is_empty() {
                    return body.into_element();
                }
                let tail_options = options_of(tail);
                let tail_rows = tail
                    .iter()
                    .enumerate()
                    .map(|(i, r)| look.build(&footer, r, tail_options.get(i).copied().flatten()));
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .child(body)
                    .children(tail_rows)
                    .into_element()
            })
        })
        .named("command-list")
        .into_element()
    }
}

/// What building a row reads.
struct Rows<'a> {
    cmd: Command,
    list: &'a ListModel,
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
                .overflow(Overflow::Hidden)
                .padding_x(4.0)
                .when(first, |d| d.padding_top(4.0))
                .when(last, |d| d.padding_bottom(4.0))
                .when(!grouped(section), |d| d.padding_x(0.0))
        };
        match *r {
            Row::Heading { section } => {
                let heading = sections
                    .get(section)
                    .and_then(CommandSection::heading)
                    .unwrap_or_default();
                group_part(section, true, false).child(
                    div()
                        .named(names.heading)
                        .padding_x(8.0)
                        .padding_y(6.0)
                        .text_size(12.0)
                        .line_height(16.0)
                        .font_weight(500)
                        .text_color(t.muted_foreground)
                        .child(heading.to_owned()),
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
                let group = sections.get(section).and_then(CommandSection::heading);
                group_part(section, first, last).child((self.row)(it, self.item_row(it, group, set)))
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

    /// An item's row: the accent while highlighted, dimmed while disabled.
    fn item_row(&self, item: &CommandItem, group: Option<&str>, set: Option<(usize, usize)>) -> Div {
        let on = !item.disabled && self.on == Some(item.value.as_str());
        let t = theme();
        let d = command_item_row()
            .named(self.names.item)
            .when(item.disabled, |d| d.opacity(0.5))
            .when(on, |d| d.background(t.accent).text_color(t.accent_foreground));
        self.cmd.option(d, self.list, item, group, on, set)
    }
}

pub(crate) fn command_empty_box(message: &str) -> Div {
    div()
        .padding_y(24.0)
        .text_align(TextAlign::Center)
        .text_size(14.0)
        .line_height(20.0)
        .child(message.to_owned())
}

/// The look of an item: `relative flex cursor-default items-center gap-2
/// rounded-sm px-2 py-1.5 text-sm select-none`.
pub(crate) fn command_item_row() -> Div {
    div()
        .position(Position::Relative)
        .display(Display::Flex)
        .cursor(Cursor::Default)
        .align_items(Align::Center)
        .gap(8.0)
        .radius(6.0)
        .padding_x(8.0)
        .padding_y(6.0)
        .text_size(14.0)
        .line_height(20.0)
}

/// `bg-border -mx-1 h-px`.
#[must_use]
pub fn command_separator() -> Div {
    let t = theme();
    div()
        .named("command-separator")
        .background(t.border)
        .margin_x(-4.0)
        .height(1.0)
}

/// `text-muted-foreground ml-auto text-xs tracking-widest`.
#[must_use]
pub fn command_shortcut(keys: &str) -> Text {
    let t = theme();
    text(keys)
        .named("command-shortcut")
        .text_color(t.muted_foreground)
        .margin_left(Length::Auto)
        .text_size(12.0)
        .line_height(16.0)
        .tracking(0.1)
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "test and example state the test reads back, not a component's"
)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use gui::{KeyEvent, Modifiers, NamedKey, Scenario, Signal, text};

    const ITEMS: usize = 10_000;

    gui::engine_local! {
        /// The context the probe found (the live frame's: a cold frame's is put back).
        [Fork] static FOUND: RefCell<Option<Rc<CommandContext>>> = RefCell::new(None);
    }

    /// A part inside the command that hands its context to the test.
    fn probe() -> Part<()> {
        Part::new((), div(), |p| {
            component(p, |cx| {
                FOUND.set(CommandContext::find().map(Rc::new));
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
            .width(400.0)
            .child(probe())
            .child(
                command_list(sections, |item, row| row.child(text(item.value.clone())))
                    .on_select(move |item: CommandItem| picked.set(item.value)),
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
            cmd.cmd.key(&ev);
        });
        s.settle().unwrap();
    }

    fn mount() -> (Scenario, CommandContext, Signal<String>, Signal<String>) {
        let search = Signal::new(String::new());
        let picked = Signal::new(String::new());
        let mut s = Scenario::measuring(move || palette(search, picked), 400, 500, 1.0).unwrap();
        s.settle().unwrap();
        let cmd = FOUND
            .with(|f| f.borrow().as_deref().cloned())
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
    fn without_a_model_a_pick_selects_nothing() {
        // shadcn-vue's Command passes modelValue '': a pick is an action.
        let (mut s, cmd, _, picked) = mount();
        press(&mut s, &cmd, NamedKey::Enter);
        assert_eq!(picked.get(), "alpha");
        let mut ts = gui::text::TextSystem::new();
        let update = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
        let selected = update
            .nodes
            .iter()
            .filter(|(_, n)| n.is_selected() == Some(true))
            .count();
        assert_eq!(selected, 0, "no item is read as selected");
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
            gui::layout::Content::Text { content, .. } if &**content == "No results found.")));
    }
}
