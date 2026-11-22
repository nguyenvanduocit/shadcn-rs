//! Dropdown menu (`ui/dropdown-menu`), and the menu primitive under it.
//!
//! The parts nest as in Vue: [`dropdown_menu`] keeps whether it is open, its
//! own (`default_open`) or the caller's (`open`, `v-model:open`), and is the
//! anchor its [`dropdown_menu_content`] opens against; the
//! [`dropdown_menu_trigger`] and the items below read it. A
//! [`dropdown_menu_sub`] is a level of its own, holding its sub trigger and
//! sub content. Closing gives focus back to the trigger, however it closes.
//!
//! ```ignore
//! dropdown_menu()
//!     .child(dropdown_menu_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(dropdown_menu_content().align(SideAlign::Start).width(224.0)
//!         .child(dropdown_menu_item("Profile", MenuItem::default()).on_select(|| …).child("Profile")))
//! ```
//!
//! Under it is reka's `Menu` ([`MenuState`], crate-private): one level of a
//! menu (the root or a submenu) with its open state, its highlighted item,
//! keyboard navigation and dismissal. Dropdown menu, context menu, menubar
//! and select build their parts on it, as their reka roots share `Menu`; a
//! root makes its level in its setup and provides it to the parts below.
//! Items join their level's list as they are laid out, in tree order, as
//! reka's collection does when items mount; the keys read that list.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gui::prelude::*;
use gui::{
    Bind, Callback, Div, Element, FocusHandle, Icon, KeyEvent, Model, NamedKey, Positioning, Side, SideAlign, Signal,
    Text, UseModel, component, deferred, div, icon, provide, text, use_context, use_model, watch,
};

use gui::style::Origin;

use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::theme::{Colors, theme};

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

/// Whether a level is open, wherever it is kept: its own signal (a submenu,
/// a select's list), a root's `v-model:open`, or a menubar's value.
pub(crate) trait OpenModel {
    /// Tracked.
    fn get(&self) -> bool;
    fn set(&self, open: bool);
}

impl OpenModel for Signal<bool> {
    fn get(&self) -> bool {
        Signal::get(self)
    }

    fn set(&self, open: bool) {
        if self.get_untracked() != open {
            Signal::set(self, open);
        }
    }
}

impl<P: 'static> OpenModel for UseModel<P, bool> {
    fn get(&self) -> bool {
        UseModel::get(self)
    }

    fn set(&self, open: bool) {
        UseModel::set(self, open);
    }
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

struct Level {
    open: Rc<dyn OpenModel>,
    highlighted: Signal<Option<String>>,
    /// This level's items on screen, in tree order (laid out last frame).
    entries: gui::Members<Entry>,
    /// The content's focus while open, so its keys reach it.
    content: FocusHandle,
    /// The trigger's focus, which closing hands back (the root's).
    trigger: FocusHandle,
    parent: Option<Weak<Level>>,
    subs: RefCell<Vec<MenuState>>,
    /// ArrowLeft and ArrowRight past the items (a menubar moves between menus).
    sideways: Option<Sideways>,
}

/// One level of a menu: the root or a submenu (reka's `Menu`). A root part
/// makes it in its setup ([`MenuState::with_open`]) and provides it; a sub
/// part makes its own with [`MenuState::sub`].
#[derive(Clone)]
pub(crate) struct MenuState(Rc<Level>);

impl Default for MenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuState {
    /// A closed level keeping its own open state.
    #[must_use]
    #[track_caller]
    pub(crate) fn new() -> Self {
        Self::with_open(Rc::new(Signal::labeled("menu open", false)), None)
    }

    /// A root level open as `open` says; `sideways` runs for ArrowLeft and
    /// ArrowRight past its items.
    #[must_use]
    #[track_caller]
    pub(crate) fn with_open(open: Rc<dyn OpenModel>, sideways: Option<Sideways>) -> Self {
        Self::level(open, None, sideways)
    }

    #[track_caller]
    fn level(open: Rc<dyn OpenModel>, parent: Option<Weak<Level>>, sideways: Option<Sideways>) -> Self {
        Self(Rc::new(Level {
            open,
            highlighted: Signal::labeled("menu highlighted", None),
            entries: gui::Members::new(),
            content: FocusHandle::new(),
            trigger: FocusHandle::new(),
            parent,
            subs: RefCell::new(Vec::new()),
            sideways,
        }))
    }

    /// A submenu of this level, made in its part's setup: it leaves the
    /// level when the part unmounts.
    #[must_use]
    #[track_caller]
    pub(crate) fn sub(&self, open: Rc<dyn OpenModel>) -> Self {
        let sub = Self::level(open, Some(Rc::downgrade(&self.0)), None);
        self.0.subs.borrow_mut().push(sub.clone());
        let (parent, me) = (Rc::downgrade(&self.0), Rc::downgrade(&sub.0));
        gui::on_cleanup(move || {
            if let Some(p) = parent.upgrade() {
                p.subs
                    .borrow_mut()
                    .retain(|s| !std::ptr::eq(Rc::as_ptr(&s.0), me.as_ptr()));
            }
        });
        sub
    }

    /// Whether this level is open (reactive).
    #[must_use]
    pub(crate) fn is_open(&self) -> bool {
        self.0.open.get()
    }

    #[must_use]
    pub(crate) fn is_highlighted(&self, id: &str) -> bool {
        self.0.highlighted.with(|h| h.as_deref() == Some(id))
    }

    /// Open this level with nothing highlighted, as a pointer opens it.
    pub(crate) fn open(&self) {
        self.0.open.set(true);
        self.0.content.focus();
    }

    /// Open this level from the keyboard, highlighting its first (or last) item.
    pub(crate) fn open_at(&self, nav: MenuNav) {
        self.0.open.set(true);
        gui::focus::set_focus(Some(self.0.content.id()), true);
        // Its items join its list as they are laid out: step once they are.
        let me = self.clone();
        gui::after_layout(move || me.highlight_step(nav));
    }

    /// Close this level and every submenu under it.
    pub(crate) fn close(&self) {
        self.close_subs();
        self.0.open.set(false);
        if self.0.highlighted.get_untracked().is_some() {
            self.0.highlighted.set(None);
        }
    }

    /// Close the whole menu and give focus back to its trigger, with a
    /// visible ring when the keyboard closed it.
    pub(crate) fn dismiss(&self, by_keyboard: bool) {
        let root = self.root();
        root.close();
        gui::focus::set_focus(Some(root.0.trigger.id()), by_keyboard);
    }

    /// The focus handle the trigger carries.
    #[must_use]
    pub(crate) fn trigger_focus(&self) -> FocusHandle {
        self.0.trigger
    }

    /// Whether this level's content, or a submenu's, holds focus.
    fn holds_focus(&self) -> bool {
        self.0.content.is_focused() || self.0.subs.borrow().iter().any(Self::holds_focus)
    }

    /// Called in a root's setup: when the menu closes with focus inside it
    /// (the caller wrote its model), focus goes back to the trigger, as
    /// reka's `FocusScope` returns it on unmount.
    pub(crate) fn return_focus_on_close(&self) {
        let me = self.clone();
        watch(move || me.is_open(), {
            let me = self.clone();
            move |open: &bool, _: &bool| {
                if !*open && me.holds_focus() {
                    gui::focus::set_focus(Some(me.0.trigger.id()), gui::focus::focus_visible());
                }
            }
        });
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

    fn close_subs(&self) {
        let subs = self.0.subs.borrow().clone();
        for sub in &subs {
            sub.close();
        }
    }

    /// Highlight `id` (pointer or keyboard); submenus of this level close and
    /// focus comes back to this level's content.
    fn highlight(&self, id: &str) {
        self.close_subs();
        if !gui::untracked(|| self.is_highlighted(id)) {
            self.0.highlighted.set(Some(id.to_owned()));
        }
        if !self.0.content.is_focused() {
            self.0.content.focus();
        }
    }

    fn enabled(&self) -> Vec<Entry> {
        self.0
            .entries
            .with(|all| all.iter().filter(|e| !e.disabled).cloned().collect())
    }

    fn highlight_step(&self, nav: MenuNav) {
        let enabled = self.enabled();
        let current = self.0.highlighted.get_untracked();
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
            .get_untracked()
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
        self.0.sideways.as_ref().is_some_and(|f| {
            f(forward);
            true
        })
    }

    /// `d` as this level's content (its items join its list as they are laid out);
    /// it takes the menu's keys, and a click outside the root closes it.
    pub(crate) fn content(&self, d: Div) -> Div {
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
        let d = d.member(
            &self.0.entries,
            Entry {
                id: id.to_owned(),
                disabled,
                action,
            },
        );
        if disabled {
            return d.disabled(true);
        }
        let (hover, click, hover_id) = (self.clone(), self.clone(), id.to_owned());
        d.on_hover(move |inside| {
            if inside {
                hover.highlight(&hover_id);
            } else if gui::untracked(|| hover.is_highlighted(&hover_id)) {
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
    /// pointer opens it, ArrowRight or Enter enters it.
    pub(crate) fn sub_trigger(&self, d: Div, id: &str, disabled: bool) -> Div {
        let Some(parent) = self.parent() else {
            return d;
        };
        let d = d.member(
            &parent.0.entries,
            Entry {
                id: id.to_owned(),
                disabled,
                action: Action::OpenSub(self.clone()),
            },
        );
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
        .on_click(move |_| click.0.open.set(true))
    }

    /// `d` as the root's trigger: a click toggles the menu, Enter, Space or
    /// ArrowDown open it on its first item.
    pub(crate) fn trigger(&self, d: Div) -> Div {
        let (click, keys) = (self.clone(), self.clone());
        d.track_focus(self.0.trigger)
            .aria_expanded(self.is_open())
            .on_click(move |_| {
                if gui::untracked(|| click.is_open()) {
                    click.close();
                } else {
                    click.open();
                }
            })
            .on_key_down(move |ev| {
                if gui::untracked(|| keys.is_open()) || !(menu_picks(ev) || ev.is(NamedKey::ArrowDown)) {
                    return false;
                }
                keys.open_at(MenuNav::First);
                true
            })
    }

    /// Highlight the first enabled item without moving focus (a select
    /// opens on it when nothing is chosen, as reka focuses it).
    pub(crate) fn highlight_first(&self) {
        let me = self.clone();
        gui::after_layout(move || me.highlight_step(MenuNav::First));
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
pub(crate) fn menu_row(t: &Colors, item: MenuItem, highlighted: bool) -> Div {
    menu_highlight(menu_row_base(t, item), t, item, highlighted)
}

/// [`menu_row`] not highlighted, for a part to highlight in its render.
pub(crate) fn menu_row_base(t: &Colors, item: MenuItem) -> Div {
    let destructive = item.variant == MenuItemVariant::Destructive;
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
        .when(item.inset, |d| d.padding_left(32.0))
        .when(item.disabled, |d| d.opacity(0.5))
        .when(destructive, |d| d.text_color(t.destructive))
}

/// A row's highlight, on top of [`menu_row_base`].
pub(crate) fn menu_highlight(d: Div, t: &Colors, item: MenuItem, highlighted: bool) -> Div {
    let destructive = item.variant == MenuItemVariant::Destructive;
    d.when(highlighted && !destructive, |d| {
        d.background(t.accent).text_color(t.accent_foreground)
    })
    .when(highlighted && destructive, |d| {
        d.background(t.destructive.opacity(if t.dark { 0.2 } else { 0.1 }))
    })
}

/// The box a checkbox or radio item's mark sits in:
/// `pointer-events-none absolute left-2 flex size-3.5 items-center justify-center`.
pub(crate) fn menu_indicator(mark: Option<gui::Element>) -> Div {
    div()
        .position(Position::Absolute)
        .left(8.0)
        .display(Display::Flex)
        .size(14.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
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
    div().size(8.0).radius(9999.0).background(color).into_element()
}

/// The content's box: `bg-popover text-popover-foreground min-w-[8rem]
/// rounded-md border p-1`, without its shadow and overflow, which differ.
pub(crate) fn menu_box(t: &Colors) -> Div {
    div()
        .background(t.popover)
        .text_color(t.popover_foreground)
        .min_width(128.0)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .padding(4.0)
}

/// `bg-border -mx-1 my-1 h-px`.
pub(crate) fn menu_separator(t: &Colors) -> Div {
    div().background(t.border).margin_x(-4.0).margin_y(4.0).height(1.0)
}

/// `text-muted-foreground ml-auto text-xs tracking-widest`.
pub(crate) fn menu_shortcut(t: &Colors, keys: &str) -> Text {
    text(keys)
        .text_color(t.muted_foreground)
        .margin_left(Length::Auto)
        .text_size(12.0)
        .line_height(16.0)
        .tracking(0.1)
}

// ── The parts every menu builds on (dropdown, context menu, menubar) ──

/// The nearest menu level, from a part's setup.
pub(crate) fn menu_level() -> Option<MenuState> {
    use_context::<MenuState>()
}

/// What a menu root is given: whether it is open.
#[derive(Clone, PartialEq, Default)]
pub struct MenuRootProps {
    open: Bind<bool>,
}

impl<P: 'static> Part<P>
where
    P: OpenProps,
{
    /// Open or not, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.bind(|b| b.with_model(open)))
    }

    /// Open from the start, when no model is given (`default-open`).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.bind(|b| b.with_default(open)))
    }

    /// Call `f` each time it opens or closes (`@update:open`).
    #[must_use]
    pub fn on_open_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.bind(|b| b.on_change(f)))
    }
}

/// Props holding a root's open state.
pub trait OpenProps {
    /// Change the open bind.
    fn bind(&mut self, f: impl FnOnce(Bind<bool>) -> Bind<bool>);
}

impl MenuRootProps {
    /// The open bind, for a root's `use_model`.
    pub(crate) fn open_bind(&self) -> &Bind<bool> {
        &self.open
    }
}

impl OpenProps for MenuRootProps {
    fn bind(&mut self, f: impl FnOnce(Bind<bool>) -> Bind<bool>) {
        self.open = f(std::mem::take(&mut self.open));
    }
}

/// A menu's root on `root`, the anchor its content opens against: it makes
/// the menu's level in its setup and provides it to the parts below.
pub(crate) fn menu_root(root: Div) -> Part<MenuRootProps> {
    Part::new(MenuRootProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = MenuState::with_open(Rc::new(use_model(props, |p| p.props.open_bind())), None);
            level.return_focus_on_close();
            provide(level);
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// A root's trigger on `root`: a click toggles the menu, Enter, Space or
/// ArrowDown open it on its first item.
pub(crate) fn menu_trigger(root: Div) -> Part<()> {
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match &level {
                    Some(l) => l.trigger(root),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// Where a menu's content opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MenuPlace {
    /// Against the root (the anchor), on `side`, lined up by `align`.
    Anchored {
        side: Side,
        align: SideAlign,
        offset: f32,
        fit_height: bool,
    },
    /// At the pointer that opened it (a context menu).
    Pointer,
    /// Beside its sub trigger, to its right (a submenu).
    Beside,
}

/// Where a context menu was opened, provided by its root.
#[derive(Clone, Copy)]
pub(crate) struct MenuPointer(pub(crate) Signal<(f32, f32)>);

/// What a menu's content is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuContentProps {
    place: MenuPlace,
}

impl Part<MenuContentProps> {
    /// On `side` of its trigger (`side="…"`; below by default).
    #[must_use]
    pub fn side(self, side: Side) -> Self {
        self.with_props(|p| {
            if let MenuPlace::Anchored { side: s, .. } = &mut p.place {
                *s = side;
            }
        })
    }

    /// Lined up with its trigger by `align` (`align="…"`).
    #[must_use]
    pub fn align(self, align: SideAlign) -> Self {
        self.with_props(|p| {
            if let MenuPlace::Anchored { align: a, .. } = &mut p.place {
                *a = align;
            }
        })
    }
}

/// A level's content on `root`: nothing while the level is closed; open,
/// floating at `place`, taking the level's keys.
pub(crate) fn menu_content(root: Div, place: MenuPlace) -> Part<MenuContentProps> {
    Part::new(MenuContentProps { place }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            let pointer = use_context::<MenuPointer>();
            move || {
                let Some(level) = &level else {
                    return Element::Empty;
                };
                if !level.is_open() {
                    return Element::Empty;
                }
                let (place, root) = props.with(|p| (p.props.place, p.slot.0.clone()));
                let placement = match place {
                    MenuPlace::Anchored {
                        side,
                        align,
                        offset,
                        fit_height,
                    } => Positioning::Anchored {
                        side,
                        align,
                        offset,
                        match_width: false,
                        fit_height,
                    },
                    MenuPlace::Pointer => {
                        let (x, y) = pointer.map_or((0.0, 0.0), |p| p.0.get());
                        Positioning::At { x, y }
                    }
                    MenuPlace::Beside => Positioning::Anchored {
                        side: Side::Right,
                        align: SideAlign::Start,
                        offset: 0.0,
                        match_width: false,
                        fit_height: false,
                    },
                };
                deferred(placement, level.content(root)).into_element()
            }
        })
    })
}

/// What an item is given: its id (what typeahead matches), its look, and
/// what picking it runs.
#[derive(Clone)]
pub struct MenuItemProps {
    id: String,
    item: MenuItem,
    on_select: Option<Callback<()>>,
}

impl Part<MenuItemProps> {
    /// Run `f` when the item is picked (`@select`); the menu then closes.
    #[must_use]
    pub fn on_select(self, f: impl Fn() + 'static) -> Self {
        self.with_props(|p| p.on_select = Some(Callback::new(move |()| f())))
    }
}

/// An item on `root`, highlighted by the pointer and the keys.
pub(crate) fn menu_item(root: Div, id: &str, item: MenuItem) -> Part<MenuItemProps> {
    let props = MenuItemProps {
        id: id.to_owned(),
        item,
        on_select: None,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            move || {
                let (id, item, on_select, root) = props.with(|p| {
                    (
                        p.props.id.clone(),
                        p.props.item,
                        p.props.on_select.clone(),
                        p.slot.0.clone(),
                    )
                });
                let Some(level) = &level else {
                    return root.into_element();
                };
                let t = theme();
                let d = menu_highlight(root, &t, item, level.is_highlighted(&id));
                level
                    .item(d, &id, item.disabled, move || {
                        if let Some(f) = &on_select {
                            f.call(());
                        }
                    })
                    .into_element()
            }
        })
    })
}

/// What a checkbox item is given: its label and whether it is checked.
#[derive(Clone)]
pub struct MenuCheckboxItemProps {
    label: String,
    disabled: bool,
    checked: Bind<bool>,
}

impl Part<MenuCheckboxItemProps> {
    /// Bound both ways to `model` (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.checked = p.checked.clone().with_model(model))
    }

    /// Showing `checked`, which the caller keeps (`:model-value`).
    #[must_use]
    pub fn checked(self, checked: bool) -> Self {
        self.with_props(|p| p.checked = p.checked.clone().with_value(checked))
    }

    /// Kept by the item, starting at `checked`.
    #[must_use]
    pub fn default_checked(self, checked: bool) -> Self {
        self.with_props(|p| p.checked = p.checked.clone().with_default(checked))
    }

    /// Call `f` with the value each pick turns it to (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.checked = p.checked.clone().on_change(f))
    }
}

/// A checkbox item on `root` (its label already in it); picking it toggles.
pub(crate) fn menu_checkbox_item(root: Div, label: &str, disabled: bool) -> Part<MenuCheckboxItemProps> {
    let props = MenuCheckboxItemProps {
        label: label.to_owned(),
        disabled,
        checked: Bind::default(),
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            let checked = use_model(props, |p| &p.props.checked);
            move || {
                let (label, disabled, root) =
                    props.with(|p| (p.props.label.clone(), p.props.disabled, p.slot.0.clone()));
                let Some(level) = &level else {
                    return root.into_element();
                };
                let t = theme();
                let item = MenuItem {
                    disabled,
                    ..MenuItem::default()
                };
                let on = checked.get();
                let d = menu_highlight(root, &t, item, level.is_highlighted(&label))
                    .role(gui::Role::MenuItemCheckBox)
                    .aria_checked(on)
                    .child(menu_indicator(on.then(menu_check)));
                level
                    .item(d, &label, disabled, move || checked.set(!checked.get_untracked()))
                    .into_element()
            }
        })
    })
}

/// What a radio group is given: the chosen value.
#[derive(Clone, PartialEq, Default)]
pub struct MenuRadioGroupProps {
    value: Bind<String>,
}

impl Part<MenuRadioGroupProps> {
    /// The chosen value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// Chosen first, when no model is given.
    #[must_use]
    pub fn default_value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(String) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

/// What a radio group's items read: its value.
#[derive(Clone, Copy)]
struct MenuRadio(UseModel<PartProps<MenuRadioGroupProps>, String>);

/// A radio group on `root`; its radio items set its value.
pub(crate) fn menu_radio_group(root: Div) -> Part<MenuRadioGroupProps> {
    Part::new(MenuRadioGroupProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(MenuRadio(use_model(props, |p| &p.props.value)));
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// What a radio item is given.
#[derive(Clone)]
pub struct MenuRadioItemProps {
    label: String,
    value: String,
    disabled: bool,
}

/// A radio item on `root` (its label already in it), setting its group to `value`.
pub(crate) fn menu_radio_item(root: Div, label: &str, value: &str, disabled: bool) -> Part<MenuRadioItemProps> {
    let props = MenuRadioItemProps {
        label: label.to_owned(),
        value: value.to_owned(),
        disabled,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            let group = use_context::<MenuRadio>();
            move || {
                let (label, value, disabled, root) = props.with(|p| {
                    (
                        p.props.label.clone(),
                        p.props.value.clone(),
                        p.props.disabled,
                        p.slot.0.clone(),
                    )
                });
                let (Some(level), Some(group)) = (&level, group) else {
                    return root.into_element();
                };
                let t = theme();
                let item = MenuItem {
                    disabled,
                    ..MenuItem::default()
                };
                let highlighted = level.is_highlighted(&label);
                let dot = if highlighted {
                    t.accent_foreground
                } else {
                    t.popover_foreground
                };
                let chosen = group.0.with(|v| *v == value);
                let d = menu_highlight(root, &t, item, highlighted)
                    .role(gui::Role::MenuItemRadio)
                    .aria_checked(chosen)
                    .child(menu_indicator(chosen.then(|| menu_dot(dot))));
                level
                    .item(d, &label, disabled, move || group.0.set(value.clone()))
                    .into_element()
            }
        })
    })
}

/// A submenu on `root`, a level of its own under the nearest one: it holds
/// its sub trigger and its sub content.
pub(crate) fn menu_sub(root: Div) -> Part<MenuRootProps> {
    Part::new(MenuRootProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            if let Some(parent) = menu_level() {
                provide(parent.sub(Rc::new(use_model(props, |p| p.props.open_bind()))));
            }
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// What a sub trigger is given.
#[derive(Clone)]
pub struct MenuSubTriggerProps {
    label: String,
    item: MenuItem,
    /// Highlighted while its submenu is open (`data-[state=open]:bg-accent`).
    open_highlights: bool,
}

/// A submenu's row on `root` (label and chevron already in it), in its
/// parent level: the pointer opens it, ArrowRight or Enter enters it.
pub(crate) fn menu_sub_trigger(
    root: Div,
    label: &str,
    item: MenuItem,
    open_highlights: bool,
) -> Part<MenuSubTriggerProps> {
    let props = MenuSubTriggerProps {
        label: label.to_owned(),
        item,
        open_highlights,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let sub = menu_level();
            move || {
                let (label, item, open_highlights, root) = props.with(|p| {
                    (
                        p.props.label.clone(),
                        p.props.item,
                        p.props.open_highlights,
                        p.slot.0.clone(),
                    )
                });
                let Some(sub) = &sub else {
                    return root.into_element();
                };
                let t = theme();
                let highlighted = (open_highlights && sub.is_open()) || sub.parent_highlighted(&label);
                sub.sub_trigger(menu_highlight(root, &t, item, highlighted), &label, item.disabled)
                    .into_element()
            }
        })
    })
}

// ── The dropdown menu ──

/// The dropdown menu's root (`DropdownMenu`): the anchor its content opens
/// against. Give it a [`dropdown_menu_trigger`] and a [`dropdown_menu_content`].
#[must_use]
pub fn dropdown_menu() -> Part<MenuRootProps> {
    menu_root(div().named("dropdown-menu").display(Display::Flex).inline(true))
}

/// Make `trigger` (usually a button) toggle the menu (`DropdownMenuTrigger as-child`).
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn dropdown_menu_trigger(trigger: impl Into<Div>) -> Part<()> {
    menu_trigger(trigger.into().named("dropdown-menu-trigger"))
}

/// The floating panel (`DropdownMenuContent`, `side-offset 4`): below the
/// trigger (the other side when only that has room), centred on it unless
/// told otherwise, no taller than the room there. Add groups, items, labels
/// and separators with `.child`.
#[must_use]
pub fn dropdown_menu_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t)
                .named("dropdown-menu-content")
                .role(gui::Role::Menu)
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .shadow(crate::popover::POPUP_SHADOW),
            Origin::Anchor,
        ),
        MenuPlace::Anchored {
            side: Side::Bottom,
            align: SideAlign::Center,
            offset: 4.0,
            fit_height: true,
        },
    )
}

#[must_use]
pub fn dropdown_menu_group() -> Div {
    div().named("dropdown-menu-group")
}

/// A group of radio items, their chosen value its own (`default_value`) or
/// the caller's (`model`, `v-model`).
#[must_use]
pub fn dropdown_menu_radio_group() -> Part<MenuRadioGroupProps> {
    menu_radio_group(div().named("dropdown-menu-radio-group"))
}

/// An item identified (and typed ahead) by `id`; add its icon, label and
/// shortcut with `.child`, what picking it does with `.on_select`. Icons
/// take `text-muted-foreground` from the caller, as shadcn's `[&_svg]` rule
/// gives them.
#[must_use]
pub fn dropdown_menu_item(id: &str, item: MenuItem) -> Part<MenuItemProps> {
    let t = theme();
    menu_item(
        menu_row_base(&t, item)
            .named("dropdown-menu-item")
            .role(gui::Role::MenuItem),
        id,
        item,
    )
}

/// A checkbox item labelled `label`, its own (`default_checked`) or bound to
/// the caller's (`model`, `v-model`); picking it toggles.
#[must_use]
pub fn dropdown_menu_checkbox_item(label: &str, disabled: bool) -> Part<MenuCheckboxItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_checkbox_item(
        menu_row_base(&t, item)
            .named("dropdown-menu-checkbox-item")
            .padding_left(32.0)
            .padding_right(8.0)
            .child(label.to_owned()),
        label,
        disabled,
    )
}

/// A radio item setting its group's value to `value`.
#[must_use]
pub fn dropdown_menu_radio_item(label: &str, value: &str, disabled: bool) -> Part<MenuRadioItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_radio_item(
        menu_row_base(&t, item)
            .named("dropdown-menu-radio-item")
            .padding_left(32.0)
            .padding_right(8.0)
            .child(label.to_owned()),
        label,
        value,
        disabled,
    )
}

/// `px-2 py-1.5 text-sm font-medium data-[inset]:pl-8`.
#[must_use]
pub fn dropdown_menu_label(label: &str, inset: bool) -> Div {
    div()
        .named("dropdown-menu-label")
        .padding_x(8.0)
        .padding_y(6.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .when(inset, |d| d.padding_left(32.0))
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

/// A submenu (`DropdownMenuSub`): give it a [`dropdown_menu_sub_trigger`]
/// and a [`dropdown_menu_sub_content`].
#[must_use]
pub fn dropdown_menu_sub() -> Part<MenuRootProps> {
    menu_sub(div().named("dropdown-menu-sub"))
}

/// The row opening its submenu, whose content opens to its right. Its
/// chevron is muted, as the item's `[&_svg]` rule makes it.
#[must_use]
pub fn dropdown_menu_sub_trigger(label: &str, item: MenuItem) -> Part<MenuSubTriggerProps> {
    let t = theme();
    menu_sub_trigger(
        menu_row_base(&t, item)
            .named("dropdown-menu-sub-trigger")
            .child(label.to_owned())
            .child(
                icon(Icon::ChevronRight)
                    .margin_left(Length::Auto)
                    .text_color(t.muted_foreground),
            ),
        label,
        item,
        false,
    )
}

/// A submenu's panel: as the content, with `shadow-lg` and `overflow-hidden`.
#[must_use]
pub fn dropdown_menu_sub_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t)
                .named("dropdown-menu-sub-content")
                .overflow(Overflow::Hidden)
                .shadow(crate::popover::LIFTED_SHADOW),
            Origin::Anchor,
        ),
        MenuPlace::Beside,
    )
}
