//! Menubar (`ui/menubar`): a row of menus, at most one open; once one is
//! open, pointing at another trigger opens that one, and ArrowLeft or
//! ArrowRight in a menu moves to its neighbour. Each menu is a [`MenuState`].

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gui::prelude::*;
use gui::{Div, Icon, Placement, Side, SideAlign, Signal, Text, div, icon};

use crate::dropdown_menu::{
    MenuItem, MenuNav, MenuState, menu_box, menu_check, menu_dot, menu_indicator, menu_row, menu_separator,
    menu_shortcut,
};
use gui::style::Origin;

use crate::motion::{popper, popper_in};
use crate::theme::theme;

/// A menubar's menus, in order. Create it once, outside the render closure,
/// and each of its menus with [`MenubarState::menu`].
#[derive(Clone, Default)]
pub struct MenubarState {
    menus: Rc<RefCell<Vec<MenuState>>>,
}

impl MenubarState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next menu of the bar.
    #[must_use]
    pub fn menu(&self) -> MenuState {
        let menu = MenuState::new();
        let index = self.menus.borrow().len();
        let bar = Rc::downgrade(&self.menus);
        menu.set_sideways(move |forward| move_sideways(&bar, index, forward));
        let (bar, me) = (Rc::downgrade(&self.menus), menu.clone());
        menu.set_on_open(move || {
            if let Some(menus) = bar.upgrade() {
                close_others(&menus.borrow(), &me);
            }
        });
        self.menus.borrow_mut().push(menu.clone());
        menu
    }

    fn any_open(&self) -> bool {
        self.menus.borrow().iter().any(MenuState::is_open)
    }
}

fn close_others(menus: &[MenuState], keep: &MenuState) {
    for m in menus {
        if !m.same(keep) {
            m.close();
        }
    }
}

/// ArrowLeft or ArrowRight past a menu's items: the neighbouring menu opens
/// on its first item (reka's `loop` is off, so the ends stay).
fn move_sideways(bar: &Weak<RefCell<Vec<MenuState>>>, index: usize, forward: bool) {
    let Some(menus) = bar.upgrade() else { return };
    let next = if forward {
        index.checked_add(1)
    } else {
        index.checked_sub(1)
    };
    let target = next.and_then(|i| menus.borrow().get(i).cloned());
    if let Some(target) = target {
        target.open_at(MenuNav::First);
    }
}

/// The bar: `bg-background flex h-9 items-center gap-1 rounded-md border p-1 shadow-xs`.
#[must_use]
pub fn menubar(_bar: &MenubarState) -> Div {
    let t = theme();
    div()
        .named("menubar")
        .bg(t.background)
        .flex()
        .h(9.0)
        .items_center()
        .gap(1.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .p(1.0)
        .shadow_xs()
}

/// A menu's trigger, `label`, opening `content` below it (`align start,
/// side-offset 8`). Pointing at it while another menu is open switches to it.
#[must_use]
pub fn menubar_trigger(bar: &MenubarState, menu: &MenuState, label: &str, content: Div) -> Div {
    let t = theme();
    let open = menu.is_open();
    let (hover_bar, hover_menu) = (bar.clone(), menu.clone());
    let d = div()
        .named("menubar-trigger")
        .flex()
        .items_center()
        .rounded_sm()
        .px(2.0)
        .py(1.0)
        .text_sm()
        .font_medium()
        .when(open, |d| d.bg(t.accent).text_color(t.accent_foreground))
        .focus(move |s| s.bg(t.accent).text_color(t.accent_foreground))
        .on_hover(move |inside| {
            if inside && !hover_menu.is_open() && hover_bar.any_open() {
                hover_menu.open();
            }
        })
        .child(label.to_owned());
    let placement = Placement::Anchored {
        side: Side::Bottom,
        align: SideAlign::Start,
        offset: 8.0,
        match_width: false,
        fit_height: false,
    };
    menu.trigger(d, placement, content)
}

/// The floating panel: `bg-popover min-w-[12rem] overflow-hidden rounded-md
/// border p-1 shadow-md`; add items with `.child`.
#[must_use]
pub fn menubar_content(menu: &MenuState) -> Div {
    let t = theme();
    menu.content(
        // No `animate-out` in its classes: it closes at once.
        popper_in(
            menu_box(&t)
                .named("menubar-content")
                .role(gui::Role::Menu)
                .min_w_px(192.0)
                .overflow_hidden()
                .shadow_md(),
            Origin::Anchor,
        ),
    )
}

#[must_use]
pub fn menubar_group() -> Div {
    div().named("menubar-group")
}

#[must_use]
pub fn menubar_radio_group() -> Div {
    div().named("menubar-radio-group")
}

/// An item identified (and typed ahead) by `id`; add its icon, label and
/// shortcut with `.child`.
#[must_use]
pub fn menubar_item(menu: &MenuState, id: &str, item: MenuItem, on_select: impl Fn() + 'static) -> Div {
    let t = theme();
    let d = menu_row(&t, item, menu.is_highlighted(id))
        .named("menubar-item")
        .role(gui::Role::MenuItem);
    menu.item(d, id, item.disabled, on_select)
}

/// A checkbox item bound to `checked` (`rounded-xs`); picking it toggles.
#[must_use]
pub fn menubar_checkbox_item(menu: &MenuState, label: &str, checked: &Signal<bool>, disabled: bool) -> Div {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    let d = menu_row(&t, item, menu.is_highlighted(label))
        .named("menubar-checkbox-item")
        .rounded_xs()
        .pl(8.0)
        .pr(2.0)
        .child(menu_indicator(checked.get().then(menu_check)))
        .child(label.to_owned());
    let checked = checked.clone();
    menu.item(d, label, disabled, move || checked.update(|c| !c))
}

/// A radio item setting `group` to `value` (`rounded-xs`).
#[must_use]
pub fn menubar_radio_item(menu: &MenuState, label: &str, value: &str, group: &Signal<String>, disabled: bool) -> Div {
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
        .named("menubar-radio-item")
        .rounded_xs()
        .pl(8.0)
        .pr(2.0)
        .child(menu_indicator((group.get() == value).then(|| menu_dot(dot))))
        .child(label.to_owned());
    let (group, value) = (group.clone(), value.to_owned());
    menu.item(d, label, disabled, move || group.set(value.clone()))
}

/// `px-2 py-1.5 text-sm font-medium data-[inset]:pl-8`.
#[must_use]
pub fn menubar_label(label: &str, inset: bool) -> Div {
    div()
        .named("menubar-label")
        .px(2.0)
        .py(1.5)
        .text_sm()
        .font_medium()
        .when(inset, |d| d.pl(8.0))
        .child(label.to_owned())
}

#[must_use]
pub fn menubar_separator() -> Div {
    menu_separator(&theme()).named("menubar-separator")
}

#[must_use]
pub fn menubar_shortcut(keys: &str) -> Text {
    menu_shortcut(&theme(), keys).named("menubar-shortcut")
}

/// The row opening `sub` (`data-[state=open]:bg-accent`, no gap); `content`
/// opens to its right.
#[must_use]
pub fn menubar_sub_trigger(sub: &MenuState, label: &str, item: MenuItem, content: Div) -> Div {
    let t = theme();
    let highlighted = sub.is_open() || sub.parent_highlighted(label);
    let d = menu_row(&t, item, highlighted)
        .named("menubar-sub-trigger")
        .gap(0.0)
        .child(label.to_owned())
        .child(icon(Icon::ChevronRight).ml_auto());
    sub.sub_trigger(d, label, item.disabled, content)
}

/// A submenu's panel: `min-w-[8rem] overflow-hidden shadow-lg`.
#[must_use]
pub fn menubar_sub_content(sub: &MenuState) -> Div {
    let t = theme();
    sub.content(popper(
        menu_box(&t).named("menubar-sub-content").overflow_hidden().shadow_lg(),
        Origin::Anchor,
    ))
}
