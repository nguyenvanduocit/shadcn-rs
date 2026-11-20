//! Dropdown menu (`ui/dropdown-menu`), and the menu primitive under it.
//!
//! [`MenuState`] is reka's `Menu`: one level of a menu (the root or a
//! submenu) with its open state, its highlighted item, keyboard navigation
//! and dismissal. Dropdown menu, context menu, menubar and select style their
//! parts on top of it, as their reka roots share `Menu`.
//!
//! Items register themselves while the content is built, in order, as reka's
//! collection does when items mount; the keys read that list.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gui::prelude::*;
use gui::{
    Div, FocusHandle, Icon, KeyEvent, NamedKey, Placement, Side, SideAlign, Signal, Text, deferred, div, icon, text,
};

use gui::style::Origin;

use crate::motion::popper;
use crate::theme::{Theme, theme};

/// A step of the highlight through a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuNav {
    Next,
    Prev,
    First,
    Last,
}

/// The step a navigation key asks for.
pub(crate) fn menu_nav_key(ev: &KeyEvent) -> Option<MenuNav> {
    [
        (NamedKey::ArrowDown, MenuNav::Next),
        (NamedKey::ArrowUp, MenuNav::Prev),
        (NamedKey::Home, MenuNav::First),
        (NamedKey::End, MenuNav::Last),
    ]
    .into_iter()
    .find_map(|(key, nav)| ev.is(key).then_some(nav))
}

/// Where the highlight goes in a list of `len` items from `current`; it stops
/// at either end (reka's `loop` is off by default).
pub(crate) fn menu_step(len: usize, current: Option<usize>, nav: MenuNav) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (nav, current) {
        (MenuNav::First, _) | (MenuNav::Next, None) => 0,
        (MenuNav::Last, _) | (MenuNav::Prev, None) => last,
        (MenuNav::Next, Some(i)) => i.saturating_add(1).min(last),
        (MenuNav::Prev, Some(i)) => i.saturating_sub(1).min(last),
    })
}

/// The next item after `current` (wrapping) whose text starts with `typed`.
pub(crate) fn menu_typeahead(texts: &[&str], current: Option<usize>, typed: &str) -> Option<usize> {
    let len = texts.len();
    let typed = typed.to_lowercase();
    let start = current.map_or(0, |i| i.saturating_add(1));
    (0..len)
        .filter_map(|k| start.checked_add(k).and_then(|i| i.checked_rem(len)))
        .find(|&i| texts.get(i).is_some_and(|t| t.to_lowercase().starts_with(&typed)))
}

/// A printable character typed without a shortcut modifier, for typeahead.
pub(crate) fn menu_typed_char(ev: &KeyEvent) -> Option<&str> {
    ev.text
        .as_deref()
        .filter(|s| s.chars().count() == 1 && s.chars().all(|c| !c.is_control() && !c.is_whitespace()))
        .filter(|_| !ev.modifiers.primary() && !ev.modifiers.control)
}

/// Enter or Space: pick the highlighted item.
pub(crate) fn menu_picks(ev: &KeyEvent) -> bool {
    ev.is(NamedKey::Enter) || ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" ")
}

#[derive(Clone)]
enum Action {
    /// Run, then close the menu.
    Select(Rc<dyn Fn()>),
    /// Run, and stay open for the next pick (a select with `multiple`).
    Toggle(Rc<dyn Fn()>),
    OpenSub(MenuState),
}

#[derive(Clone)]
struct Entry {
    id: String,
    disabled: bool,
    action: Action,
}

/// What ArrowLeft (`false`) or ArrowRight (`true`) does past a menu's items.
type Sideways = Rc<dyn Fn(bool)>;
/// What opening a menu does to its neighbours (a menubar closes them).
type OnOpen = Rc<dyn Fn()>;

struct Level {
    open: Signal<bool>,
    highlighted: Signal<Option<String>>,
    /// The items of the last content built, in order.
    entries: RefCell<Vec<Entry>>,
    /// The content's focus while open, so its keys reach it.
    content: FocusHandle,
    /// The trigger's focus, which closing hands back (the root's).
    trigger: FocusHandle,
    parent: Option<Weak<Level>>,
    subs: RefCell<Vec<MenuState>>,
    /// ArrowLeft and ArrowRight past the items (a menubar moves between menus).
    sideways: RefCell<Option<Sideways>>,
    on_open: RefCell<Option<OnOpen>>,
}

/// One level of a menu: the root or a submenu. Create it once, outside the
/// render closure; [`MenuState::sub`] creates a submenu under it.
#[derive(Clone)]
pub struct MenuState(Rc<Level>);

impl Default for MenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuState {
    #[must_use]
    pub fn new() -> Self {
        Self::level(None)
    }

    fn level(parent: Option<Weak<Level>>) -> Self {
        Self(Rc::new(Level {
            open: Signal::labeled("menu open", false),
            highlighted: Signal::labeled("menu highlighted", None),
            entries: RefCell::new(Vec::new()),
            content: FocusHandle::new(),
            trigger: FocusHandle::new(),
            parent,
            subs: RefCell::new(Vec::new()),
            sideways: RefCell::new(None),
            on_open: RefCell::new(None),
        }))
    }

    /// A submenu of this level.
    #[must_use]
    pub fn sub(&self) -> Self {
        let sub = Self::level(Some(Rc::downgrade(&self.0)));
        self.0.subs.borrow_mut().push(sub.clone());
        sub
    }

    /// Whether this level is open (reactive).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.0.open.get()
    }

    /// The highlighted item's id (reactive).
    #[must_use]
    pub fn highlighted(&self) -> Option<String> {
        self.0.highlighted.get()
    }

    #[must_use]
    pub fn is_highlighted(&self, id: &str) -> bool {
        self.0.highlighted.get().as_deref() == Some(id)
    }

    /// Open this level with nothing highlighted, as a pointer opens it.
    pub fn open(&self) {
        self.opening();
        self.0.content.focus();
    }

    /// Open this level from the keyboard, highlighting its first (or last) item.
    pub(crate) fn open_at(&self, nav: MenuNav) {
        self.opening();
        gui::focus::set_focus(Some(self.0.content.id()), true);
        self.highlight_step(nav);
    }

    /// Close this level and every submenu under it.
    pub fn close(&self) {
        for sub in self.0.subs.borrow().iter() {
            sub.close();
        }
        if self.0.open.get() {
            self.0.open.set(false);
        }
        if self.0.highlighted.get().is_some() {
            self.0.highlighted.set(None);
        }
    }

    /// Close the whole menu and give focus back to its trigger, with a
    /// visible ring when the keyboard closed it.
    pub fn dismiss(&self, by_keyboard: bool) {
        let root = self.root();
        root.close();
        gui::focus::set_focus(Some(root.0.trigger.id()), by_keyboard);
    }

    /// The focus handle the trigger carries.
    #[must_use]
    pub fn trigger_focus(&self) -> FocusHandle {
        self.0.trigger
    }

    /// Whether this submenu's row, `id`, is highlighted in its parent (reactive).
    pub(crate) fn parent_highlighted(&self, id: &str) -> bool {
        self.parent().is_some_and(|p| p.is_highlighted(id))
    }

    fn root(&self) -> Self {
        self.parent().map_or_else(|| self.clone(), |p| p.root())
    }

    fn parent(&self) -> Option<Self> {
        self.0.parent.as_ref().and_then(Weak::upgrade).map(Self)
    }

    pub(crate) fn set_sideways(&self, f: impl Fn(bool) + 'static) {
        self.0.sideways.replace(Some(Rc::new(f)));
    }

    pub(crate) fn set_on_open(&self, f: impl Fn() + 'static) {
        self.0.on_open.replace(Some(Rc::new(f)));
    }

    /// Whether `self` and `other` are the same menu.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    fn opening(&self) {
        let on_open = self.0.on_open.borrow().clone();
        if let Some(f) = on_open {
            f();
        }
        self.0.open.set(true);
    }

    fn close_subs(&self) {
        for sub in self.0.subs.borrow().iter() {
            sub.close();
        }
    }

    /// Highlight `id` (pointer or keyboard); submenus of this level close and
    /// focus comes back to this level's content.
    fn highlight(&self, id: &str) {
        self.close_subs();
        if !self.is_highlighted(id) {
            self.0.highlighted.set(Some(id.to_owned()));
        }
        if !self.0.content.is_focused() {
            self.0.content.focus();
        }
    }

    fn enabled(&self) -> Vec<Entry> {
        self.0
            .entries
            .borrow()
            .iter()
            .filter(|e| !e.disabled)
            .cloned()
            .collect()
    }

    fn highlight_step(&self, nav: MenuNav) {
        let enabled = self.enabled();
        let current = self.0.highlighted.get();
        let at = current.and_then(|h| enabled.iter().position(|e| e.id == h));
        if let Some(e) = menu_step(enabled.len(), at, nav).and_then(|i| enabled.get(i)) {
            self.close_subs();
            self.0.highlighted.set(Some(e.id.clone()));
        }
    }

    fn activate(&self, entry: &Entry, by_keyboard: bool) {
        match &entry.action {
            Action::Select(f) => {
                f();
                self.dismiss(by_keyboard);
            }
            Action::Toggle(f) => f(),
            Action::OpenSub(sub) => sub.open_at(MenuNav::First),
        }
    }

    fn key(&self, ev: &KeyEvent) -> bool {
        if let Some(nav) = menu_nav_key(ev) {
            self.highlight_step(nav);
            return true;
        }
        let enabled = self.enabled();
        let current = self
            .0
            .highlighted
            .get()
            .and_then(|h| enabled.iter().position(|e| e.id == h));
        let entry = current.and_then(|i| enabled.get(i));
        if menu_picks(ev) {
            if let Some(e) = entry {
                self.activate(e, true);
            }
            return true;
        }
        if ev.is(NamedKey::ArrowRight) {
            if let Some(Entry {
                action: Action::OpenSub(sub),
                ..
            }) = entry
            {
                sub.open_at(MenuNav::First);
                return true;
            }
            return self.go_sideways(true);
        }
        if ev.is(NamedKey::ArrowLeft) {
            if let Some(parent) = self.parent() {
                self.close();
                gui::focus::set_focus(Some(parent.0.content.id()), true);
                return true;
            }
            return self.go_sideways(false);
        }
        if ev.is(NamedKey::Escape) {
            self.dismiss(true);
            return true;
        }
        if let Some(typed) = menu_typed_char(ev) {
            let texts: Vec<&str> = enabled.iter().map(|e| e.id.as_str()).collect();
            if let Some(e) = menu_typeahead(&texts, current, typed).and_then(|i| enabled.get(i)) {
                self.highlight(&e.id);
            }
            return true;
        }
        false
    }

    fn go_sideways(&self, forward: bool) -> bool {
        let f = self.0.sideways.borrow().clone();
        f.is_some_and(|f| {
            f(forward);
            true
        })
    }

    /// `d` as this level's content: the items built after it register here;
    /// it takes the menu's keys, and a click outside the root closes it.
    pub(crate) fn content(&self, d: Div) -> Div {
        self.0.entries.borrow_mut().clear();
        let keys = self.clone();
        let d = d.track_focus(self.0.content).on_key_down(move |ev| keys.key(ev));
        if self.0.parent.is_some() {
            return d;
        }
        let outside = self.clone();
        d.on_click_outside(move || outside.dismiss(false))
    }

    /// `d` as an item of this level: the pointer highlights it, a click or
    /// Enter runs `on_select` and closes the menu.
    pub(crate) fn item(&self, d: Div, id: &str, disabled: bool, on_select: impl Fn() + 'static) -> Div {
        self.entry(d, id, disabled, Rc::new(on_select), true)
    }

    /// An item that leaves the menu open once `on_select` has run.
    pub(crate) fn toggle_item(&self, d: Div, id: &str, disabled: bool, on_select: impl Fn() + 'static) -> Div {
        self.entry(d, id, disabled, Rc::new(on_select), false)
    }

    fn entry(&self, d: Div, id: &str, disabled: bool, on_select: Rc<dyn Fn()>, closes: bool) -> Div {
        let action = if closes {
            Action::Select(on_select.clone())
        } else {
            Action::Toggle(on_select.clone())
        };
        self.0.entries.borrow_mut().push(Entry {
            id: id.to_owned(),
            disabled,
            action,
        });
        if disabled {
            return d.disabled(true);
        }
        let (hover, click, hover_id) = (self.clone(), self.clone(), id.to_owned());
        d.on_hover(move |inside| {
            if inside {
                hover.highlight(&hover_id);
            } else if hover.is_highlighted(&hover_id) {
                hover.0.highlighted.set(None);
            }
        })
        .on_click(move |_| {
            on_select();
            if closes {
                click.dismiss(false);
            }
        })
    }

    /// `d` as the trigger of this submenu, inside its parent level: the
    /// pointer opens it, ArrowRight or Enter enters it; `content` floats beside it.
    pub(crate) fn sub_trigger(&self, d: Div, id: &str, disabled: bool, content: Div) -> Div {
        let Some(parent) = self.parent() else {
            return d;
        };
        parent.0.entries.borrow_mut().push(Entry {
            id: id.to_owned(),
            disabled,
            action: Action::OpenSub(self.clone()),
        });
        if disabled {
            return d.disabled(true);
        }
        let (hover, click, hover_id) = (self.clone(), self.clone(), id.to_owned());
        let hover_parent = parent;
        d.on_hover(move |inside| {
            if inside {
                hover_parent.highlight(&hover_id);
                hover.0.open.set(true);
            }
        })
        .on_click(move |_| {
            if !click.is_open() {
                click.0.open.set(true);
            }
        })
        .child(self.is_open().then(|| {
            deferred(
                Placement::Anchored {
                    side: Side::Right,
                    align: SideAlign::Start,
                    offset: 0.0,
                    match_width: false,
                    fit_height: false,
                },
                content,
            )
        }))
    }

    /// `d` as the root's trigger: a click toggles the menu, Enter, Space or
    /// ArrowDown open it on its first item; `content` floats at `placement`.
    pub(crate) fn trigger(&self, d: Div, placement: Placement, content: Div) -> Div {
        let (click, keys) = (self.clone(), self.clone());
        d.track_focus(self.0.trigger)
            .on_click(move |_| {
                if click.is_open() {
                    click.close();
                } else {
                    click.open();
                }
            })
            .on_key_down(move |ev| {
                if keys.is_open() || !(menu_picks(ev) || ev.is(NamedKey::ArrowDown)) {
                    return false;
                }
                keys.open_at(MenuNav::First);
                true
            })
            .child(self.is_open().then(|| deferred(placement, content)))
    }

    /// Highlight the first enabled item without moving focus (a select
    /// opens on it when nothing is chosen, as reka focuses it).
    pub(crate) fn highlight_first(&self) {
        self.highlight_step(MenuNav::First);
    }

    /// Highlight `id` without moving focus (a select opens on its value).
    pub(crate) fn set_highlighted(&self, id: Option<String>) {
        self.0.highlighted.set(id);
    }
}

/// What an item looks like and whether it takes input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MenuItem {
    /// Indented to line up with checkbox and radio items (`inset`).
    pub inset: bool,
    pub disabled: bool,
    pub variant: MenuItemVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MenuItemVariant {
    #[default]
    Default,
    Destructive,
}

impl MenuItem {
    pub const DISABLED: Self = Self {
        inset: false,
        disabled: true,
        variant: MenuItemVariant::Default,
    };
    pub const INSET: Self = Self {
        inset: true,
        disabled: false,
        variant: MenuItemVariant::Default,
    };
    pub const DESTRUCTIVE: Self = Self {
        inset: false,
        disabled: false,
        variant: MenuItemVariant::Destructive,
    };
}

/// The look every menu row shares: `relative flex cursor-default items-center
/// gap-2 rounded-sm px-2 py-1.5 text-sm select-none data-[disabled]:opacity-50
/// data-[inset]:pl-8`, highlighted as `focus:bg-accent focus:text-accent-foreground`
/// (the destructive variant: `text-destructive focus:bg-destructive/10`, `/20` in dark).
pub(crate) fn menu_row(t: &Theme, item: MenuItem, highlighted: bool) -> Div {
    let destructive = item.variant == MenuItemVariant::Destructive;
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
        .when(item.inset, |d| d.pl(8.0))
        .when(item.disabled, |d| d.opacity(0.5))
        .when(destructive, |d| d.text_color(t.destructive))
        .when(highlighted && !destructive, |d| {
            d.bg(t.accent).text_color(t.accent_foreground)
        })
        .when(highlighted && destructive, |d| {
            d.bg(t.destructive.opacity(if t.dark { 0.2 } else { 0.1 }))
        })
}

/// The box a checkbox or radio item's mark sits in:
/// `pointer-events-none absolute left-2 flex size-3.5 items-center justify-center`.
pub(crate) fn menu_indicator(mark: Option<gui::Element>) -> Div {
    div()
        .absolute()
        .left(2.0)
        .flex()
        .size(3.5)
        .items_center()
        .justify_center()
        .pass_through()
        .child(mark)
}

/// The check of a checked checkbox item (`Check class="size-4"`).
pub(crate) fn menu_check() -> gui::Element {
    icon(Icon::Check).into_element()
}

/// The dot of a chosen radio item (`Circle class="size-2 fill-current"`): a
/// filled circle in the text colour.
pub(crate) fn menu_dot(color: gui::Color) -> gui::Element {
    div().size(2.0).rounded_full().bg(color).into_element()
}

/// The content's box: `bg-popover text-popover-foreground min-w-[8rem]
/// rounded-md border p-1`, without its shadow and overflow, which differ.
pub(crate) fn menu_box(t: &Theme) -> Div {
    div()
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .min_w_px(128.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .p(1.0)
}

/// `bg-border -mx-1 my-1 h-px`.
pub(crate) fn menu_separator(t: &Theme) -> Div {
    div().bg(t.border).mx(-1.0).my(1.0).h_px(1.0)
}

/// `text-muted-foreground ml-auto text-xs tracking-widest`.
pub(crate) fn menu_shortcut(t: &Theme, keys: &str) -> Text {
    text(keys)
        .text_color(t.muted_foreground)
        .ml_auto()
        .text_xs()
        .tracking_widest()
}

/// The trigger: `trigger` (usually a button) toggles `content`, which opens
/// on its `side` (the other one when only that has room), lined up by
/// `align` (`side-offset 4`), and no taller than the room there.
#[must_use]
pub fn dropdown_menu_trigger(menu: &MenuState, trigger: Div, side: Side, align: SideAlign, content: Div) -> Div {
    let placement = Placement::Anchored {
        side,
        align,
        offset: 4.0,
        match_width: false,
        fit_height: true,
    };
    menu.trigger(trigger.named("dropdown-menu-trigger"), placement, content)
}

/// The floating panel; add groups, items, labels and separators with `.child`.
#[must_use]
pub fn dropdown_menu_content(menu: &MenuState) -> Div {
    let t = theme();
    menu.content(popper(
        menu_box(&t)
            .named("dropdown-menu-content")
            .overflow_y_scroll()
            .shadow_md(),
        Origin::Anchor,
    ))
}

#[must_use]
pub fn dropdown_menu_group() -> Div {
    div().named("dropdown-menu-group")
}

#[must_use]
pub fn dropdown_menu_radio_group() -> Div {
    div().named("dropdown-menu-radio-group")
}

/// An item of `menu` identified (and typed ahead) by `id`; add its icon,
/// label and shortcut with `.child`. Icons take `text-muted-foreground` from
/// the caller, as shadcn's `[&_svg]` rule gives them.
#[must_use]
pub fn dropdown_menu_item(menu: &MenuState, id: &str, item: MenuItem, on_select: impl Fn() + 'static) -> Div {
    let t = theme();
    let d = menu_row(&t, item, menu.is_highlighted(id)).named("dropdown-menu-item");
    menu.item(d, id, item.disabled, on_select)
}

/// A checkbox item bound to `checked` (`v-model`); picking it toggles.
#[must_use]
pub fn dropdown_menu_checkbox_item(menu: &MenuState, label: &str, checked: &Signal<bool>, disabled: bool) -> Div {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    let on = checked.get();
    let d = menu_row(&t, item, menu.is_highlighted(label))
        .named("dropdown-menu-checkbox-item")
        .pl(8.0)
        .pr(2.0)
        .child(menu_indicator(on.then(menu_check)))
        .child(label.to_owned());
    let checked = checked.clone();
    menu.item(d, label, disabled, move || checked.update(|c| !c))
}

/// A radio item setting `group` (`v-model` of the radio group) to `value`.
#[must_use]
pub fn dropdown_menu_radio_item(
    menu: &MenuState,
    label: &str,
    value: &str,
    group: &Signal<String>,
    disabled: bool,
) -> Div {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    let highlighted = menu.is_highlighted(label);
    let dot = if highlighted {
        t.accent_foreground
    } else {
        t.popover_foreground
    };
    let chosen = group.get() == value;
    let d = menu_row(&t, item, highlighted)
        .named("dropdown-menu-radio-item")
        .pl(8.0)
        .pr(2.0)
        .child(menu_indicator(chosen.then(|| menu_dot(dot))))
        .child(label.to_owned());
    let (group, value) = (group.clone(), value.to_owned());
    menu.item(d, label, disabled, move || group.set(value.clone()))
}

/// `px-2 py-1.5 text-sm font-medium data-[inset]:pl-8`.
#[must_use]
pub fn dropdown_menu_label(label: &str, inset: bool) -> Div {
    div()
        .named("dropdown-menu-label")
        .px(2.0)
        .py(1.5)
        .text_sm()
        .font_medium()
        .when(inset, |d| d.pl(8.0))
        .child(label.to_owned())
}

#[must_use]
pub fn dropdown_menu_separator() -> Div {
    menu_separator(&theme()).named("dropdown-menu-separator")
}

#[must_use]
pub fn dropdown_menu_shortcut(keys: &str) -> Text {
    menu_shortcut(&theme(), keys).named("dropdown-menu-shortcut")
}

/// The row opening `sub`; `content` (a [`dropdown_menu_sub_content`]) opens
/// to its right. Its chevron is muted, as the item's `[&_svg]` rule makes it.
#[must_use]
pub fn dropdown_menu_sub_trigger(sub: &MenuState, label: &str, item: MenuItem, content: Div) -> Div {
    let t = theme();
    let highlighted = sub.parent_highlighted(label);
    let d = menu_row(&t, item, highlighted)
        .named("dropdown-menu-sub-trigger")
        .child(label.to_owned())
        .child(icon(Icon::ChevronRight).ml_auto().text_color(t.muted_foreground));
    sub.sub_trigger(d, label, item.disabled, content)
}

/// A submenu's panel: as the content, with `shadow-lg` and `overflow-hidden`.
#[must_use]
pub fn dropdown_menu_sub_content(sub: &MenuState) -> Div {
    let t = theme();
    sub.content(popper(
        menu_box(&t)
            .named("dropdown-menu-sub-content")
            .overflow_hidden()
            .shadow_lg(),
        Origin::Anchor,
    ))
}
