//! Context menu (`ui/context-menu`): a menu opened by a right click, at the
//! pointer. Its behaviour is the menu primitive's ([`MenuState`]).

use std::ops::Deref;

use gui::prelude::*;
use gui::{Div, Icon, Placement, Signal, Text, div, icon};

use crate::dropdown_menu::{
    MenuItem, MenuState, menu_box, menu_check, menu_dot, menu_indicator, menu_row, menu_separator, menu_shortcut,
};
use gui::style::Origin;

use crate::motion::popper;
use crate::theme::theme;

/// A context menu: its menu and where the right click opened it. Create it
/// once, outside the render closure; it reads as its [`MenuState`] for items.
#[derive(Clone)]
pub struct ContextMenuState {
    menu: MenuState,
    at: Signal<(f32, f32)>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenuState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            menu: MenuState::new(),
            at: Signal::labeled("context menu at", (0.0, 0.0)),
        }
    }
}

impl Deref for ContextMenuState {
    type Target = MenuState;

    fn deref(&self) -> &MenuState {
        &self.menu
    }
}

/// The area a right click opens `content` in, at the pointer; a press of the
/// primary button inside it closes the menu, as it is outside the content.
#[must_use]
pub fn context_menu_trigger(menu: &ContextMenuState, area: Div, content: Div) -> Div {
    let (open, close) = (menu.clone(), menu.clone());
    let (x, y) = menu.at.get();
    area.named("context-menu-trigger")
        .on_secondary_click(move |ev| {
            open.close();
            open.at.set((ev.x, ev.y));
            open.menu.open();
        })
        .on_mouse_down(move |_| close.close())
        .child(menu.is_open().then(|| gui::deferred(Placement::At { x, y }, content)))
}

/// The floating panel: `bg-popover min-w-[8rem] overflow-y-auto rounded-md
/// border p-1 shadow-md`; add items with `.child`.
#[must_use]
pub fn context_menu_content(menu: &MenuState) -> Div {
    let t = theme();
    menu.content(popper(
        menu_box(&t)
            .named("context-menu-content")
            .role(gui::Role::Menu)
            .overflow_y_scroll()
            .shadow_md(),
        Origin::CENTER,
    ))
}

#[must_use]
pub fn context_menu_group() -> Div {
    div().named("context-menu-group")
}

#[must_use]
pub fn context_menu_radio_group() -> Div {
    div().named("context-menu-radio-group")
}

/// An item identified (and typed ahead) by `id`; add its icon, label and
/// shortcut with `.child`.
#[must_use]
pub fn context_menu_item(menu: &MenuState, id: &str, item: MenuItem, on_select: impl Fn() + 'static) -> Div {
    let t = theme();
    let d = menu_row(&t, item, menu.is_highlighted(id))
        .named("context-menu-item")
        .role(gui::Role::MenuItem);
    menu.item(d, id, item.disabled, on_select)
}

/// A checkbox item bound to `checked`; picking it toggles.
#[must_use]
pub fn context_menu_checkbox_item(menu: &MenuState, label: &str, checked: &Signal<bool>, disabled: bool) -> Div {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    let d = menu_row(&t, item, menu.is_highlighted(label))
        .named("context-menu-checkbox-item")
        .pl(8.0)
        .pr(2.0)
        .child(menu_indicator(checked.get().then(menu_check)))
        .child(label.to_owned());
    let checked = checked.clone();
    menu.item(d, label, disabled, move || checked.update(|c| !c))
}

/// A radio item setting `group` to `value`.
#[must_use]
pub fn context_menu_radio_item(
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
    let d = menu_row(&t, item, highlighted)
        .named("context-menu-radio-item")
        .pl(8.0)
        .pr(2.0)
        .child(menu_indicator((group.get() == value).then(|| menu_dot(dot))))
        .child(label.to_owned());
    let (group, value) = (group.clone(), value.to_owned());
    menu.item(d, label, disabled, move || group.set(value.clone()))
}

/// `text-foreground px-2 py-1.5 text-sm font-medium data-[inset]:pl-8`.
#[must_use]
pub fn context_menu_label(label: &str, inset: bool) -> Div {
    let t = theme();
    div()
        .named("context-menu-label")
        .text_color(t.foreground)
        .px(2.0)
        .py(1.5)
        .text_sm()
        .font_medium()
        .when(inset, |d| d.pl(8.0))
        .child(label.to_owned())
}

#[must_use]
pub fn context_menu_separator() -> Div {
    menu_separator(&theme()).named("context-menu-separator")
}

#[must_use]
pub fn context_menu_shortcut(keys: &str) -> Text {
    menu_shortcut(&theme(), keys).named("context-menu-shortcut")
}

/// The row opening `sub` (`data-[state=open]:bg-accent`, no gap, chevron in
/// the text colour); `content` opens to its right.
#[must_use]
pub fn context_menu_sub_trigger(sub: &MenuState, label: &str, item: MenuItem, content: Div) -> Div {
    let t = theme();
    let highlighted = sub.is_open() || sub.parent_highlighted(label);
    let d = menu_row(&t, item, highlighted)
        .named("context-menu-sub-trigger")
        .gap(0.0)
        .child(label.to_owned())
        .child(icon(Icon::ChevronRight).ml_auto());
    sub.sub_trigger(d, label, item.disabled, content)
}

/// A submenu's panel: as the content, with `shadow-lg` and `overflow-hidden`.
#[must_use]
pub fn context_menu_sub_content(sub: &MenuState) -> Div {
    let t = theme();
    sub.content(popper(
        menu_box(&t)
            .named("context-menu-sub-content")
            .overflow_hidden()
            .shadow_lg(),
        Origin::CENTER,
    ))
}
