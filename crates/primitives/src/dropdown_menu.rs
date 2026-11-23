//! A menu opened by a button (reka's `DropdownMenu`; research §8,
//! "DropdownMenu"), on the [`Menu`] engine.
//!
//! - **Trigger** (`DropdownMenuTrigger`): a click (on release, the primary
//!   button, not with Control: Control and a click is the secondary button
//!   on macOS) toggles the menu, opened with no item highlighted; Enter and
//!   Space toggle it, opened on its first item; ArrowDown opens it on its
//!   first item and never closes it; ArrowUp does nothing. It says
//!   `aria-haspopup=menu` and `aria-expanded`, and controls the content
//!   while it is open. A disabled trigger takes no focus and opens nothing.
//! - **Content**: the [`Menu`]'s, named by the trigger, modal by default.
//!   Below the trigger, centred (reka's popper defaults); shadcn moves it
//!   4 px off and lines it up as its caller says.
//! - **Closing** gives focus back to the trigger, the engine's focus scope
//!   rule (ADR-0024): unless something else took focus meanwhile (a layer
//!   opened by the same click), or a press outside closed it. A modal menu
//!   gives focus back after a primary press outside (the trigger included:
//!   it is outside, the page below is inert), not after a secondary one;
//!   one that is not modal keeps focus where any press outside put it.
//!   This is reka's `hasInteractedOutside` table (research §8, the
//!   wrapper table).
//! - Opened by a click, it opens on release, so pressing the trigger and
//!   releasing on an item picks nothing (reka's).
//! - A press on the trigger while the menu is open closes it, once: reka
//!   keeps the press from counting as outside and lets the click toggle;
//!   here the page below a modal menu takes no press (ADR-0024), so the
//!   press closes it as outside and no click follows.
//! - **Filter** ([`DropdownMenuFilter`], reka's `DropdownMenuFilter`; shadcn
//!   uses none): a search field in a content, a `searchbox` naming the
//!   highlighted item as its active descendant. While it holds focus the
//!   highlight is the level's own, not focus: the pointer over an item and
//!   the vertical arrows, Home and End move it (focus stays in the field,
//!   the highlight brought into view), Enter picks it, Escape empties a
//!   field that holds text (the menu stays) and closes the menu when it is
//!   empty; while an input method composes, every key is its own. The
//!   pointer coming into the content focuses it. A submenu closed by its
//!   key gives focus back to its parent's filter, its sub trigger
//!   highlighted; focus moving from a submenu to its parent's filter does
//!   not close it. The field does not filter the items: the caller does,
//!   reading its text. Asked to, it takes focus once laid out, in a submenu
//!   only while the user is at the keyboard (reka's `autoFocus`, 1 ms after
//!   mount).

use gui::element::Node;
use gui::{Div, HasPopup, KeyEvent, NamedKey, Role, TextField};

use crate::availability::Availability;
use crate::menu::{Menu, MenuOptions, OpenFocus, OpenModel};

/// A dropdown menu: made once in its root's setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropdownMenu {
    menu: Menu,
}

impl DropdownMenu {
    /// A dropdown menu open as `open` says (its owner decided once by the
    /// caller's `Bind`, ADR-0025 §1), set up as `options` (reka: modal).
    #[must_use]
    #[track_caller]
    pub fn new(open: impl OpenModel + 'static, options: MenuOptions) -> Self {
        Self {
            menu: Menu::new(open, options),
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

    fn toggle(self, focus: OpenFocus) {
        if self.menu.is_open_untracked() {
            self.menu.close_all();
        } else {
            self.menu.open(focus);
        }
    }

    /// The keys the trigger takes (its whole table: Enter, Space, ArrowDown).
    fn key(self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        if m.control || m.alt || m.command {
            return false;
        }
        if crate::keymap::activates(ev) {
            self.toggle(OpenFocus::First);
            return true;
        }
        if ev.is(NamedKey::ArrowDown) {
            self.menu.open(OpenFocus::First);
            return true;
        }
        false
    }

    /// `d` as the trigger, usable as `availability` says. Called in render:
    /// it reads whether the menu is open (tracked). Sets the node's click
    /// and key handlers.
    #[must_use]
    pub fn trigger(self, d: Div, availability: Availability) -> Div {
        let menu = self.menu;
        let open = menu.is_open();
        let d = menu
            .link()
            .trigger(d.track_focus(menu.trigger_focus()), open)
            .aria_has_popup(HasPopup::Menu)
            .aria_expanded(open);
        let d = availability.apply(d);
        if !availability.actionable() {
            return d;
        }
        d.on_click(move |ev| {
            if !ev.modifiers.control {
                self.toggle(OpenFocus::Content);
            }
        })
        .on_key_down(move |ev| self.key(ev))
    }
}

/// A search field in a menu's content (reka's `DropdownMenuFilter`): made
/// once in its part's setup, `Copy`.
#[derive(Clone, Copy)]
pub struct DropdownMenuFilter {
    menu: Menu,
    field: TextField,
    disabled: bool,
}

impl std::fmt::Debug for DropdownMenuFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DropdownMenuFilter").finish_non_exhaustive()
    }
}

impl DropdownMenuFilter {
    /// The filter of level `menu` (the root's or a submenu's), editing
    /// `field` (the caller reads its text to filter the items), taking
    /// focus once laid out when `auto_focus` (in a submenu, only while the
    /// user is at the keyboard); `disabled` takes no input. Called in the
    /// part's setup: the level knows its filter until the part goes.
    #[must_use]
    #[track_caller]
    pub fn new(menu: Menu, field: TextField, auto_focus: bool, disabled: bool) -> Self {
        menu.use_filter(field.focus);
        let in_sub = menu.parent().is_some();
        if auto_focus && (!in_sub || OpenFocus::by_modality() == OpenFocus::First) {
            let focus = field.focus;
            gui::after_layout(move || {
                if menu.is_open_untracked() {
                    gui::focus::set_focus(Some(focus.id()), false);
                }
            });
        }
        Self { menu, field, disabled }
    }

    /// `d` (a text input on the field) as the filter: a `searchbox` naming
    /// the highlighted item (tracked), taking the menu's keys before its
    /// own editing.
    #[must_use]
    pub fn field<K>(self, d: Node<K>) -> Node<K> {
        let d = d.role(Role::SearchInput).active_descendant(self.menu.active_node());
        if self.disabled {
            return d.disabled(true);
        }
        let (menu, field) = (self.menu, self.field);
        d.on_key_down(move |ev| menu.filter_key(ev, &field))
    }
}
