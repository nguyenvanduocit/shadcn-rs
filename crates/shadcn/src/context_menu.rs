//! Context menu (`ui/context-menu`): a menu opened by a right click (or a
//! long press, or the context-menu key), at the pointer. Its behaviour is
//! `primitives`' ([`primitives::context_menu::ContextMenu`] on the menu
//! engine); the parts draw it (see [`crate::dropdown_menu`]).
//!
//! The parts nest as in Vue: [`context_menu`] keeps whether it is open
//! (`on_open_change` hears it) and where it opened; it holds the
//! [`context_menu_trigger`] area and the [`context_menu_content`].

use gui::prelude::*;
use gui::{Div, Icon, Text, component, div, icon, provide, use_context, use_model};

use primitives::Availability;
use primitives::context_menu::ContextMenu;

use crate::dropdown_menu::{
    MenuCheckboxItemProps, MenuContentProps, MenuItem, MenuItemProps, MenuPlace, MenuRadioGroupProps,
    MenuRadioItemProps, MenuRootProps, MenuSubTriggerProps, menu_box, menu_checkbox_item, menu_content, menu_group,
    menu_item, menu_item_separator, menu_radio_group, menu_radio_item, menu_row_base, menu_shortcut, menu_sub,
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
                let (options, delay) = props.with_untracked(|p| (p.props.options(), p.props.press_open_delay()));
                let menu = ContextMenu::new(use_model(props, |p| p.props.open_bind()), options);
                let menu = match delay {
                    Some(d) => menu.press_open_delay(d),
                    None => menu,
                };
                provide(menu);
                provide(menu.menu());
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

/// The area a right click, a long press or the context-menu key opens the
/// menu in, at the pointer (`ContextMenuTrigger as-child`); a primary press
/// inside it closes the menu, as it is outside the content.
#[must_use]
pub fn context_menu_trigger(area: Div) -> Part<()> {
    Part::new((), area.named("context-menu-trigger"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let menu = use_context::<ContextMenu>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match menu {
                    Some(m) => m.area(root, Availability::Enabled),
                    None => root,
                }
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
    menu_group(div().named("context-menu-group"))
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
    menu_item(menu_row_base(&t, item).named("context-menu-item"), id, item)
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
    menu_item_separator(&theme()).named("context-menu-separator")
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
/// chevron in the text colour); the content opens on the inline end.
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
