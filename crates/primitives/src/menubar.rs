//! A row of menus, at most one open (reka's `Menubar`; research §8,
//! "Menubar"), on the [`Menu`] engine and [`Roving`].
//!
//! - **Value.** The bar holds the open menu's value, empty for none (reka's
//!   `modelValue`), kept by the caller or by the bar from a default. Each
//!   [`MenubarMenu`] is open while the value is its own; it gets a value of
//!   its own when not given one (reka's `useId`).
//! - **One Tab stop.** The triggers are a [`Roving`] group
//!   ([`Keymap::MENUBAR`]): ArrowRight and ArrowLeft move between them (they
//!   swap right to left), skipping disabled ones, stopping at the ends
//!   unless the bar wraps (reka's `loop`, off); Home, PageUp, End, PageDown
//!   reach the ends; none moves with Control, Alt, Command or Shift.
//! - **Trigger keys** (`MenubarTrigger`): Enter and Space toggle at the
//!   bar's level (any menu open: all close; none: this one opens on its
//!   first item); ArrowDown opens this one on its first item; ArrowUp does
//!   nothing.
//! - **Pointer.** A primary press (not with Control) on a trigger opens its
//!   menu as it goes down, so pressing, dragging to an item and releasing
//!   picks it (the item's [`gui::Div::on_release`]); a press on the trigger
//!   of the open menu closes it. While a
//!   menu is open, the pointer entering another usable trigger opens that
//!   one at once; while none is, hovering does nothing. reka also opens a
//!   disabled trigger on hover; here it does not (research §8, "Dễ sót").
//! - **In a content**, the arrow toward the inline end on a plain item, and
//!   the other one anywhere, open the next and previous menus (wrapping as
//!   the bar does, skipping disabled triggers) with no item highlighted;
//!   on a sub trigger the inline-end arrow opens its submenu. From a
//!   submenu's plain item the inline-end arrow opens the next menu: reka
//!   hard-codes ArrowRight there, so under RTL it both closed the submenu
//!   and moved on; here it follows the direction (research §4 item 13).
//! - **Focus.** Opened from a trigger's keys, the content focuses its first
//!   item; opened by the pointer or moved to by the arrows, the content
//!   itself. The menus are never modal; focus leaving closes the open one,
//!   but focus on a trigger of the bar does not. Closing gives focus back
//!   to the trigger (the engine's focus scope) unless another menu opened in
//!   its place or a press outside closed it.
//! - **AT.** The bar is a `menubar` (no orientation, as reka); a trigger
//!   is a `menuitem` with `aria-haspopup=menu` and `aria-expanded`,
//!   controlling its content while open; the content is named by its
//!   trigger.

use std::rc::Rc;

use gui::{DismissReason, Div, HasPopup, KeyEvent, NamedKey, Role, Signal, UseModel};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::use_direction;
use crate::keymap::{Keymap, Wrap};
use crate::menu::{Menu, MenuOptions, OpenFocus, OpenModel};
use crate::roving::Roving;

/// The bar's value: which menu is open, `""` for none.
pub trait BarValue {
    /// Whether `value`'s menu is open (tracked).
    fn is(&self, value: &str) -> bool;
    /// The open menu's value, untracked.
    fn current(&self) -> String;
    fn set(&self, value: String);
}

impl BarValue for Signal<String> {
    fn is(&self, value: &str) -> bool {
        self.with(|v| v == value)
    }

    fn current(&self) -> String {
        self.get_untracked()
    }

    fn set(&self, value: String) {
        if self.with_untracked(|v| *v != value) {
            Signal::set(self, value);
        }
    }
}

impl<P: 'static> BarValue for UseModel<P, String> {
    fn is(&self, value: &str) -> bool {
        self.with(|v| v == value)
    }

    fn current(&self) -> String {
        self.get_untracked()
    }

    fn set(&self, value: String) {
        UseModel::set(self, value);
    }
}

struct Bar {
    value: Rc<dyn BarValue>,
    wrap: Wrap,
    /// The bar's menus by value, registered in their setups.
    menus: gui::Registry<(Rc<str>, Menu)>,
}

/// A menubar: made once in its root's setup, `Copy`.
#[derive(Clone, Copy)]
pub struct Menubar {
    bar: Signal<Bar>,
    triggers: Roving<Rc<str>>,
}

impl PartialEq for Menubar {
    fn eq(&self, other: &Self) -> bool {
        self.bar == other.bar
    }
}

impl std::fmt::Debug for Menubar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Menubar").finish_non_exhaustive()
    }
}

impl Menubar {
    /// A bar whose open menu `value` holds, wrapping as `wrap` says (reka's
    /// `loop`, off: [`Wrap::Stop`]); called in the root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(value: impl BarValue + 'static, wrap: Wrap) -> Self {
        Self {
            bar: Signal::new(Bar {
                value: Rc::new(value),
                wrap,
                menus: gui::Registry::new(),
            }),
            triggers: Roving::new(Keymap::MENUBAR.wrap(wrap), use_direction(None)),
        }
    }

    fn value(self) -> Rc<dyn BarValue> {
        self.bar.with_untracked(|b| Rc::clone(&b.value))
    }

    /// Whether any menu is open (tracked).
    #[must_use]
    pub fn any_open(self) -> bool {
        !self.value().is("")
    }

    fn any_open_now(self) -> bool {
        !self.value().current().is_empty()
    }

    /// Close every menu.
    pub fn close(self) {
        self.value().set(String::new());
    }

    /// `d` as the bar: a `menubar`.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(Role::MenuBar)
    }

    /// The menu after (`forward`) or before the one of value `from`, among
    /// usable triggers, wrapping as the bar does.
    fn neighbour(self, from: &str, forward: bool) -> Option<Menu> {
        let wrap = self.bar.with_untracked(|b| b.wrap);
        let target = self.triggers.items().with(|list| {
            let usable = |i: usize| list.get(i).is_some_and(|t| t.availability.focusable());
            let at = list.iter().position(|t| &*t.value == from)?;
            let len = list.len();
            let around = matches!(wrap, Wrap::Around);
            let step = |k: usize| -> Option<usize> {
                if forward {
                    let i = at.checked_add(k)?;
                    if i < len {
                        Some(i)
                    } else if around {
                        i.checked_sub(len)
                    } else {
                        None
                    }
                } else {
                    match at.checked_sub(k) {
                        Some(i) => Some(i),
                        None if around => at.checked_add(len).and_then(|i| i.checked_sub(k)),
                        None => None,
                    }
                }
            };
            let found = (1..len).filter_map(step).find(|&i| usable(i));
            found.and_then(|i| list.get(i)).map(|t| Rc::clone(&t.value))
        })?;
        gui::untracked(|| self.menu(&target))
    }

    /// The menu of `value`, while its part is mounted (tracked).
    #[must_use]
    pub fn menu(self, value: &str) -> Option<Menu> {
        self.bar
            .with_untracked(|b| b.menus)
            .with(|mut l| l.find(|(v, _)| &**v == value).map(|(_, m)| *m))
    }

    /// Whether the focus `f` is on one of the bar's triggers.
    fn on_trigger(self, f: gui::focus::FocusId) -> bool {
        self.triggers.items().index_of(f).is_some()
    }
}

/// One menu of a bar is open while the bar's value is its own.
struct MenuOpen {
    bar: Menubar,
    me: Rc<str>,
}

impl OpenModel for MenuOpen {
    fn get(&self) -> bool {
        self.bar.value().is(&self.me)
    }

    fn set(&self, open: bool) {
        let value = self.bar.value();
        if open {
            value.set(self.me.to_string());
        } else if value.current() == *self.me {
            value.set(String::new());
        }
    }
}

/// One menu of a bar (reka's `MenubarMenu`): made once in its part's
/// setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenubarMenu {
    bar: Menubar,
    menu: Menu,
    value: Signal<Rc<str>>,
}

impl MenubarMenu {
    /// The menu of `value` in `bar` (never modal, as reka's), its other
    /// options as `options` says; called in the menu part's setup.
    #[must_use]
    #[track_caller]
    pub fn new(bar: Menubar, value: impl Into<Rc<str>>, options: MenuOptions) -> Self {
        let value: Rc<str> = value.into();
        let wrap = bar.bar.with_untracked(|b| b.wrap);
        let menu = Menu::new(
            MenuOpen {
                bar,
                me: Rc::clone(&value),
            },
            options.modal(false).wrap(wrap),
        );
        bar.bar.with_untracked(|b| b.menus).register((Rc::clone(&value), menu));
        let from = Rc::clone(&value);
        menu.set_sideways(move |forward| {
            if let Some(next) = bar.neighbour(&from, forward) {
                next.open(OpenFocus::Content);
            }
        });
        // Focus on a trigger of the bar is not outside its menus.
        menu.veto(move |reason| {
            matches!(reason, DismissReason::FocusOutside)
                && gui::untracked(gui::focus::focused).is_some_and(|f| bar.on_trigger(f))
        });
        Self {
            bar,
            menu,
            value: Signal::new(value),
        }
    }

    /// Its menu: the content and items are the engine's parts.
    #[must_use]
    pub fn menu(self) -> Menu {
        self.menu
    }

    /// Whether it is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.menu.is_open()
    }

    /// Its value in the bar.
    #[must_use]
    pub fn value(self) -> Rc<str> {
        self.value.get_untracked()
    }

    /// The trigger's own keys, then the bar's arrows.
    fn key(self, ev: &KeyEvent, usable: bool) -> bool {
        let m = ev.modifiers;
        if usable && !(m.control || m.alt || m.command) {
            if crate::keymap::activates(ev) {
                if self.bar.any_open_now() {
                    self.bar.close();
                } else {
                    self.menu.open(OpenFocus::First);
                }
                return true;
            }
            if ev.is(NamedKey::ArrowDown) {
                self.menu.open(OpenFocus::First);
                return true;
            }
        }
        self.bar.triggers.key(self.menu.trigger_focus(), ev).consumed()
    }

    /// `d` as this menu's trigger, labelled `label` for the bar's order,
    /// usable as `availability` says. Called in render: it reads whether
    /// the menu is open (tracked). Sets the node's key, press and hover
    /// handlers.
    #[must_use]
    pub fn trigger(self, d: Div, label: impl Into<Rc<str>>, availability: Availability) -> Div {
        let open = self.menu.is_open();
        let item = Item::new(self.value(), self.menu.trigger_focus())
            .label(label)
            .availability(availability);
        let d = self
            .menu
            .link()
            .trigger(self.bar.triggers.item(d, item), open)
            .role(Role::MenuItem)
            .aria_has_popup(HasPopup::Menu)
            .aria_expanded(open);
        let usable = availability.actionable();
        let d = d.on_key_down(move |ev| self.key(ev, usable));
        if !usable {
            return d;
        }
        d.on_mouse_down(move |ev| {
            if ev.modifiers.control {
                return;
            }
            if self.menu.is_open_untracked() {
                self.bar.close();
            } else {
                self.menu.open(OpenFocus::Content);
            }
        })
        .on_hover(move |inside| {
            if inside && self.bar.any_open_now() && !self.menu.is_open_untracked() {
                self.menu.open(OpenFocus::Content);
            }
        })
    }
}
