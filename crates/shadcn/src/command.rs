//! Command (`ui/command`): a search box over a list of items, filtered as
//! you type, highlighted by the pointer and the arrow keys, picked with a
//! click or Enter. Combobox's list is a command list too.
//!
//! As in shadcn-vue's `Command.vue`, every item is known by its value: an
//! item hides when its value does not contain the search (case-insensitive),
//! a group hides when all its items do, and the empty state shows when a
//! search hides them all.

use std::cell::RefCell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Div, Icon, KeyEvent, NamedKey, Text, TextField, div, icon, text, text_input};

use crate::dropdown_menu::{menu_nav_key, menu_step};
use crate::theme::theme;

/// The highlighted value, and the search it was highlighted under: once the
/// search changes, the first item is highlighted instead (reka highlights the
/// first item on every search).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Highlight {
    value: Option<String>,
    search: String,
}

#[derive(Clone)]
struct Entry {
    value: String,
    on_select: Rc<dyn Fn()>,
}

/// A command's search and highlight. Create it once, outside the render closure.
#[derive(Clone)]
pub struct CommandState {
    /// The search box (`filterState.search`).
    pub search: TextField,
    highlight: gui::Signal<Highlight>,
    /// The visible, enabled items of the frame being built, in order.
    entries: Rc<RefCell<Vec<Entry>>>,
    /// Every item's value, this frame's so far and the last frame's.
    values: Rc<RefCell<Vec<String>>>,
    last_values: Rc<RefCell<Vec<String>>>,
}

impl Default for CommandState {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether an item valued `value` shows under `search`.
fn matches(value: &str, search: &str) -> bool {
    search.is_empty() || value.to_lowercase().contains(&search.to_lowercase())
}

impl CommandState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            search: TextField::new(""),
            highlight: gui::Signal::labeled("command highlight", Highlight::default()),
            entries: Rc::new(RefCell::new(Vec::new())),
            values: Rc::new(RefCell::new(Vec::new())),
            last_values: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Start a frame: the items built after this register anew.
    pub(crate) fn begin(&self) {
        let values = std::mem::take(&mut *self.values.borrow_mut());
        *self.last_values.borrow_mut() = values;
        self.entries.borrow_mut().clear();
    }

    /// Whether any known item shows under the current search.
    pub(crate) fn any_visible(&self) -> bool {
        let search = self.search.text();
        let visible = |vs: &RefCell<Vec<String>>| vs.borrow().iter().any(|v| matches(v, &search));
        visible(&self.values) || visible(&self.last_values)
    }

    /// The highlighted value (reactive).
    #[must_use]
    pub fn highlighted(&self) -> Option<String> {
        let h = self.highlight.get();
        if h.search == self.search.text() {
            h.value
        } else {
            self.entries.borrow().first().map(|e| e.value.clone())
        }
    }

    /// Highlight `value` under the current search.
    fn highlight(&self, value: Option<String>) {
        let next = Highlight {
            value,
            search: self.search.text(),
        };
        if self.highlight.get() != next {
            self.highlight.set(next);
        }
    }

    /// Highlight `value`, or the first item when it is `None` (a combobox
    /// opening on its chosen value).
    pub(crate) fn highlight_or_first(&self, value: Option<String>) {
        let first = || self.entries.borrow().first().map(|e| e.value.clone());
        self.highlight(value.or_else(first));
    }

    fn pick(&self, entry: &Entry) {
        (entry.on_select)();
        self.search.set_text("");
    }

    /// The list's keys: arrows and Home/End move the highlight, Enter picks.
    /// [`command`] takes them itself; a search field outside it (reka's
    /// `ListboxFilter`) hands them over.
    pub fn key(&self, ev: &KeyEvent) -> bool {
        let entries = self.entries.borrow().clone();
        let current = self
            .highlighted()
            .and_then(|h| entries.iter().position(|e| e.value == h));
        if let Some(nav) = menu_nav_key(ev) {
            if let Some(e) = menu_step(entries.len(), current, nav).and_then(|i| entries.get(i)) {
                self.highlight(Some(e.value.clone()));
            }
            return true;
        }
        if ev.is(NamedKey::Enter) {
            if let Some(e) = current.and_then(|i| entries.get(i)) {
                self.pick(e);
            }
            return true;
        }
        false
    }

    /// `d` as an item valued `value`, or `None` when the search hides it.
    pub(crate) fn item(&self, d: Div, value: &str, disabled: bool, on_select: impl Fn() + 'static) -> Option<Div> {
        self.values.borrow_mut().push(value.to_owned());
        if !matches(value, &self.search.text()) {
            return None;
        }
        if disabled {
            return Some(d.disabled(true).opacity(0.5));
        }
        let t = theme();
        let on = match self.highlight.get() {
            h if h.search == self.search.text() => h.value.as_deref() == Some(value),
            _ => self.entries.borrow().is_empty(),
        };
        let entry = Entry {
            value: value.to_owned(),
            on_select: Rc::new(on_select),
        };
        self.entries.borrow_mut().push(entry.clone());
        let (hover, click) = (self.clone(), self.clone());
        let (hovered, picked) = (entry.value.clone(), entry);
        Some(
            d.when(on, |d| d.bg(t.accent).text_color(t.accent_foreground))
                .on_hover(move |inside| {
                    if inside {
                        hover.highlight(Some(hovered.clone()));
                    }
                })
                .on_click(move |_| click.pick(&picked)),
        )
    }
}

/// The root: `bg-popover text-popover-foreground flex h-full w-full flex-col
/// overflow-hidden rounded-md`; it takes the list's keys.
#[must_use]
pub fn command(cmd: &CommandState) -> Div {
    let t = theme();
    cmd.begin();
    let keys = cmd.clone();
    div()
        .named("command")
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .flex()
        .h_full()
        .w_full()
        .flex_col()
        .overflow_hidden()
        .rounded_md()
        .on_key_down(move |ev| keys.key(ev))
}

/// The search box: `flex h-9 items-center gap-2 border-b px-3`, a faded
/// search icon and the input (`h-10 py-3 text-sm`, no outline).
#[must_use]
pub fn command_input(cmd: &CommandState, placeholder: &str) -> Div {
    let t = theme();
    div()
        .named("command-input-wrapper")
        .flex()
        .h(9.0)
        .items_center()
        .gap(2.0)
        .border_b()
        .border_color(t.border)
        .px(3.0)
        .child(icon(Icon::Search).shrink_0().opacity(0.5))
        .child(
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
}

/// The scrolling list: `max-h-[300px] overflow-y-auto`.
#[must_use]
pub fn command_list() -> Div {
    div()
        .named("command-list")
        .max_h_px(300.0)
        .overflow_x_hidden()
        .overflow_y_scroll()
}

/// `py-6 text-center text-sm`, shown when a search hides every item.
#[must_use]
pub fn command_empty(cmd: &CommandState, message: &str) -> Option<Div> {
    let shown = !cmd.search.text().is_empty() && !cmd.any_visible();
    shown.then(|| command_empty_box(message).named("command-empty"))
}

pub(crate) fn command_empty_box(message: &str) -> Div {
    div().py(6.0).text_center().text_sm().child(message.to_owned())
}

/// A group (`text-foreground overflow-hidden p-1`) under an optional
/// `heading` (`px-2 py-1.5 text-xs font-medium text-muted-foreground`);
/// `None` when the search hides all its items.
#[must_use]
pub fn command_group(heading: Option<&str>, items: impl IntoIterator<Item = Option<Div>>) -> Option<Div> {
    let t = theme();
    let items: Vec<Div> = items.into_iter().flatten().collect();
    (!items.is_empty()).then(|| {
        div()
            .named("command-group")
            .text_color(t.foreground)
            .overflow_hidden()
            .p(1.0)
            .child(heading.map(|h| {
                div()
                    .named("command-group-heading")
                    .px(2.0)
                    .py(1.5)
                    .text_xs()
                    .font_medium()
                    .text_color(t.muted_foreground)
                    .child(h.to_owned())
            }))
            .children(items)
    })
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

/// An item valued `value` (what the search matches), or `None` when the
/// search hides it; add its icon, label and shortcut with `.map(|i| i.child(…))`.
/// Icons take `text-muted-foreground` from the caller, as shadcn's `[&_svg]` rule gives them.
#[must_use]
pub fn command_item(cmd: &CommandState, value: &str, disabled: bool, on_select: impl Fn() + 'static) -> Option<Div> {
    cmd.item(command_item_row().named("command-item"), value, disabled, on_select)
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
