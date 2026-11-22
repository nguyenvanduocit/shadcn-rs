//! Context menu (`ui/context-menu`): a menu opened by a right click, at the
//! pointer. Its behaviour is the menu primitive's (see [`crate::dropdown_menu`]).
//!
//! The parts nest as in Vue: [`context_menu`] keeps whether it is open
//! (`on_open_change` hears it) and where it opened; it holds the
//! [`context_menu_trigger`] area and the [`context_menu_content`].

use std::rc::Rc;

use gui::prelude::*;
use gui::{Div, Icon, Signal, Text, component, div, icon, provide, use_model};

use crate::dropdown_menu::{
    MenuCheckboxItemProps, MenuContentProps, MenuItem, MenuItemProps, MenuPlace, MenuPointer, MenuRadioGroupProps,
    MenuRadioItemProps, MenuRootProps, MenuState, MenuSubTriggerProps, menu_box, menu_checkbox_item, menu_content,
    menu_item, menu_level, menu_radio_group, menu_radio_item, menu_row_base, menu_separator, menu_shortcut, menu_sub,
    menu_sub_trigger,
};
use gui::style::Origin;

use crate::motion::popper;
use crate::part::Part;
use crate::theme::theme;

/// The context menu's root (`ContextMenu`): give it a
/// [`context_menu_trigger`] area and a [`context_menu_content`].
#[must_use]
pub fn context_menu() -> Part<MenuRootProps> {
    Part::new(
        MenuRootProps::default(),
        div().named("context-menu").display(Display::Flex),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let level = MenuState::with_open(Rc::new(use_model(props, |p| p.props.open_bind())), None);
                level.return_focus_on_close();
                provide(level);
                provide(MenuPointer(Signal::labeled("context menu at", (0.0, 0.0))));
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

/// The area a right click opens the menu in, at the pointer
/// (`ContextMenuTrigger as-child`); a press of the primary button inside it
/// closes the menu, as it is outside the content.
#[must_use]
pub fn context_menu_trigger(area: Div) -> Part<()> {
    Part::new((), area.named("context-menu-trigger"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            let at = gui::use_context::<MenuPointer>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let (Some(level), Some(at)) = (level.clone(), at) else {
                    return root.into_element();
                };
                let close = level.clone();
                root.on_secondary_click(move |ev| {
                    level.close();
                    at.0.set((ev.x, ev.y));
                    level.open();
                })
                .on_mouse_down(move |_| close.close())
                .into_element()
            }
        })
    })
}

/// The floating panel at the pointer: `bg-popover min-w-[8rem]
/// overflow-y-auto rounded-md border p-1 shadow-md`; add items with `.child`.
#[must_use]
pub fn context_menu_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t)
                .named("context-menu-content")
                .role(gui::Role::Menu)
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .shadow(crate::popover::POPUP_SHADOW),
            Origin::CENTER,
        ),
        MenuPlace::Pointer,
    )
}

#[must_use]
pub fn context_menu_group() -> Div {
    div().named("context-menu-group")
}

/// A group of radio items, their chosen value its own (`default_value`) or
/// the caller's (`model`).
#[must_use]
pub fn context_menu_radio_group() -> Part<MenuRadioGroupProps> {
    menu_radio_group(div().named("context-menu-radio-group"))
}

/// An item identified (and typed ahead) by `id`; add its icon, label and
/// shortcut with `.child`, what picking it does with `.on_select`.
#[must_use]
pub fn context_menu_item(id: &str, item: MenuItem) -> Part<MenuItemProps> {
    let t = theme();
    menu_item(
        menu_row_base(&t, item)
            .named("context-menu-item")
            .role(gui::Role::MenuItem),
        id,
        item,
    )
}

/// A checkbox item labelled `label`; picking it toggles.
#[must_use]
pub fn context_menu_checkbox_item(label: &str, disabled: bool) -> Part<MenuCheckboxItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_checkbox_item(
        menu_row_base(&t, item)
            .named("context-menu-checkbox-item")
            .padding_left(32.0)
            .padding_right(8.0)
            .child(label.to_owned()),
        label,
        disabled,
    )
}

/// A radio item setting its group's value to `value`.
#[must_use]
pub fn context_menu_radio_item(label: &str, value: &str, disabled: bool) -> Part<MenuRadioItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_radio_item(
        menu_row_base(&t, item)
            .named("context-menu-radio-item")
            .padding_left(32.0)
            .padding_right(8.0)
            .child(label.to_owned()),
        label,
        value,
        disabled,
    )
}

/// `text-foreground px-2 py-1.5 text-sm font-medium data-[inset]:pl-8`.
#[must_use]
pub fn context_menu_label(label: &str, inset: bool) -> Div {
    let t = theme();
    div()
        .named("context-menu-label")
        .text_color(t.foreground)
        .padding_x(8.0)
        .padding_y(6.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .when(inset, |d| d.padding_left(32.0))
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

/// A submenu (`ContextMenuSub`): give it a [`context_menu_sub_trigger`] and
/// a [`context_menu_sub_content`].
#[must_use]
pub fn context_menu_sub() -> Part<MenuRootProps> {
    menu_sub(div().named("context-menu-sub"))
}

/// The row opening its submenu (`data-[state=open]:bg-accent`, no gap,
/// chevron in the text colour); the content opens to its right.
#[must_use]
pub fn context_menu_sub_trigger(label: &str, item: MenuItem) -> Part<MenuSubTriggerProps> {
    let t = theme();
    menu_sub_trigger(
        menu_row_base(&t, item)
            .named("context-menu-sub-trigger")
            .gap(0.0)
            .child(label.to_owned())
            .child(icon(Icon::ChevronRight).margin_left(Length::Auto)),
        label,
        item,
        true,
    )
}

/// A submenu's panel: as the content, with `shadow-lg` and `overflow-hidden`.
#[must_use]
pub fn context_menu_sub_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t)
                .named("context-menu-sub-content")
                .overflow(Overflow::Hidden)
                .shadow(crate::popover::LIFTED_SHADOW),
            Origin::CENTER,
        ),
        MenuPlace::Beside,
    )
}
