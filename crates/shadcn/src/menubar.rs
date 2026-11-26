//! Menubar (`ui/menubar`): a row of menus, at most one open; once one is
//! open, pointing at another trigger opens that one, and ArrowLeft or
//! ArrowRight in a menu moves to its neighbour. Its behaviour is
//! `primitives`' ([`primitives::menubar::Menubar`] on the menu engine); the
//! parts draw it (see [`crate::dropdown_menu`]).
//!
//! The parts nest as in Vue: [`menubar`] keeps which menu is open (reka's
//! `modelValue`, the open menu's value or `""`), its own or the caller's
//! (`model`); each [`menubar_menu`] holds a trigger and its content, and is
//! open while the bar's value is its own.

use gui::prelude::*;
use gui::{Bind, Div, Icon, Model, Text, component, div, icon, provide, use_context, use_model};

use primitives::Availability;
use primitives::Wrap;
use primitives::menu::MenuOptions;
use primitives::menubar::{Menubar, MenubarMenu};

use crate::dropdown_menu::{
    MenuCheckboxItemProps, MenuContentProps, MenuFamily, MenuItem, MenuItemProps, MenuPlace, MenuRadioGroupProps,
    MenuRadioItemProps, MenuRootProps, MenuSubTriggerProps, menu_box, menu_checkbox_item, menu_content, menu_group,
    menu_item, menu_item_separator, menu_radio_group, menu_radio_item, menu_row_base, menu_shortcut, menu_sub,
    menu_sub_trigger,
};
use gui::style::Origin;
use gui::{Side, SideAlign};

use crate::focus::StateLook;
use crate::motion::{popper, popper_in};
use crate::part::{Part, part_root};
use crate::theme::theme;

/// What [`menubar`] is given: which menu is open (`""`: none), and what
/// the keys do past the first or last trigger.
#[derive(Clone, PartialEq)]
pub struct MenubarProps {
    value: Bind<String>,
    wrap: Wrap,
}

impl Default for MenubarProps {
    fn default() -> Self {
        Self {
            value: Bind::default(),
            wrap: Wrap::Stop,
        }
    }
}

impl Part<MenubarProps> {
    /// The open menu's value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The menu open first, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// What the arrows do past the first or last trigger, past a content's
    /// first or last item, and from the last menu to the first (reka's
    /// `loop`: off, [`Wrap::Stop`]).
    /// Read once, as the bar is made.
    #[must_use]
    pub fn wrap(self, wrap: Wrap) -> Self {
        self.with_props(|p| p.wrap = wrap)
    }

    /// Call `f` with the open menu's value each time it changes.
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

/// The bar: `bg-background flex h-9 items-center gap-1 rounded-md border p-1
/// shadow-xs`; give it [`menubar_menu`]s.
#[must_use]
pub fn menubar() -> Part<MenubarProps> {
    let t = theme();
    let root = div()
        .named("menubar")
        .background(t.background)
        .display(Display::Flex)
        .height(36.0)
        .align_items(Align::Center)
        .gap(4.0)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .padding(4.0)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: gui::Color::BLACK.opacity(0.05),
        });
    Part::new(MenubarProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let wrap = props.with_untracked(|p| p.props.wrap);
            let bar = Menubar::new(use_model(props, |p| &p.props.value), wrap);
            provide(bar);
            move || bar.root(props.with(|p| p.slot.0.clone())).into_element()
        })
    })
}

/// What [`menubar_menu`] is given: its value in the bar.
#[derive(Clone, PartialEq, Default)]
pub struct MenubarMenuProps {
    value: Option<String>,
}

impl Part<MenubarMenuProps> {
    /// Its value in the bar's `model` (one of its own otherwise).
    #[must_use]
    pub fn value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = Some(value))
    }
}

/// Values for menus not given one (reka's `useId`).
static NEXT_MENU: gui::engine::IdKind = gui::engine::IdKind::new("menubar menu");

/// One menu of the bar (`MenubarMenu`): the anchor its content opens
/// against. Give it a [`menubar_trigger`] and a [`menubar_content`].
#[must_use]
pub fn menubar_menu() -> Part<MenubarMenuProps> {
    Part::new(
        MenubarMenuProps::default(),
        div().named("menubar-menu").display(Display::Flex),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let me = props.with_untracked(|p| p.props.value.clone()).unwrap_or_else(|| {
                    let n = gui::engine::unique_id(&NEXT_MENU);
                    format!("menubar-menu-{n}")
                });
                if let Some(bar) = use_context::<Menubar>() {
                    let menu = MenubarMenu::new(bar, me, MenuOptions::DEFAULT);
                    provide(menu);
                    provide(menu.menu());
                }
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

/// A menu's trigger, `label` (`align start, side-offset 8` for its content),
/// lit while focused or open. Pressing it opens its menu; pointing at it
/// while another menu is open switches to it.
#[must_use]
#[track_caller]
pub fn menubar_trigger(label: &str) -> Part<MenubarTriggerProps> {
    let root = div()
        .named("menubar-trigger")
        .display(Display::Flex)
        .align_items(Align::Center)
        .radius(6.0)
        .padding_x(8.0)
        .padding_y(4.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .child(label.to_owned());
    let props = MenubarTriggerProps {
        label: label.to_owned(),
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let menu = part_root::<_, MenubarMenu>(props, "menubar_trigger", "menubar_menu");
            move || {
                let (label, root) = props.with(|p| (p.props.label.clone(), p.slot.0.clone()));
                let Some(menu) = menu else {
                    return root.into_element();
                };
                let t = theme();
                let d = root.with_interaction(|d, i| {
                    let lit = move || i.focused() || menu.is_open();
                    d.background_while(lit, t.accent)
                        .text_color_while(lit, t.accent_foreground)
                });
                menu.trigger(d, label, Availability::Enabled).into_element()
            }
        })
    })
}

/// What [`menubar_trigger`] is given: its label, what the bar's order reads.
#[derive(Clone, Debug, PartialEq)]
pub struct MenubarTriggerProps {
    label: String,
}

/// The floating panel below its trigger (`align start, side-offset 8`):
/// `bg-popover min-w-[12rem] overflow-hidden rounded-md border p-1
/// shadow-md`; add items with `.child`.
#[must_use]
#[track_caller]
pub fn menubar_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        // No `animate-out` in its classes: it closes at once.
        popper_in(
            menu_box(&t)
                .named("menubar-content")
                .min_width(192.0)
                .overflow(Overflow::Hidden)
                .shadow(crate::popover::POPUP_SHADOW),
            Origin::Anchor,
        ),
        MenuPlace::Anchored {
            side: Side::Bottom,
            align: SideAlign::Start,
            offset: 8.0,
            fit_height: false,
        },
        MenuFamily::Menubar,
    )
}

#[must_use]
pub fn menubar_group() -> Div {
    menu_group(div().named("menubar-group"))
}

/// A group of radio items, their chosen value its own (`default_value`) or
/// the caller's (`model`).
#[must_use]
pub fn menubar_radio_group() -> Part<MenuRadioGroupProps> {
    menu_radio_group(div().named("menubar-radio-group"))
}

/// An item identified (and typed ahead) by `id`; add its icon, label and
/// shortcut with `.child`, what picking it does with `.on_select`.
#[must_use]
#[track_caller]
pub fn menubar_item(id: &str, item: MenuItem) -> Part<MenuItemProps> {
    let t = theme();
    menu_item(
        menu_row_base(&t, item).named("menubar-item"),
        id,
        item,
        MenuFamily::Menubar,
    )
}

/// A checkbox item labelled `label` (`rounded-xs`); picking it toggles.
#[must_use]
#[track_caller]
pub fn menubar_checkbox_item(label: &str, disabled: bool) -> Part<MenuCheckboxItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_checkbox_item(
        menu_row_base(&t, item)
            .named("menubar-checkbox-item")
            .radius(2.0)
            .padding_left(32.0)
            .padding_right(8.0)
            .child(label.to_owned()),
        label,
        disabled,
        MenuFamily::Menubar,
    )
}

/// A radio item setting its group's value to `value` (`rounded-xs`).
#[must_use]
#[track_caller]
pub fn menubar_radio_item(label: &str, value: &str, disabled: bool) -> Part<MenuRadioItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_radio_item(
        menu_row_base(&t, item)
            .named("menubar-radio-item")
            .radius(2.0)
            .padding_left(32.0)
            .padding_right(8.0)
            .child(label.to_owned()),
        label,
        value,
        disabled,
        MenuFamily::Menubar,
    )
}

/// `px-2 py-1.5 text-sm font-medium data-[inset]:pl-8`.
#[must_use]
pub fn menubar_label(label: &str, inset: bool) -> Div {
    div()
        .named("menubar-label")
        .padding_x(8.0)
        .padding_y(6.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .when(inset, |d| d.padding_left(32.0))
        .child(label.to_owned())
}

#[must_use]
pub fn menubar_separator() -> Div {
    menu_item_separator(&theme()).named("menubar-separator")
}

#[must_use]
pub fn menubar_shortcut(keys: &str) -> Text {
    menu_shortcut(&theme(), keys).named("menubar-shortcut")
}

/// A submenu (`MenubarSub`): give it a [`menubar_sub_trigger`] and a
/// [`menubar_sub_content`].
#[must_use]
#[track_caller]
pub fn menubar_sub() -> Part<MenuRootProps> {
    menu_sub(div().named("menubar-sub"), MenuFamily::Menubar)
}

/// The row opening its submenu (`data-[state=open]:bg-accent`, no gap); the
/// content opens on the inline end.
#[must_use]
#[track_caller]
pub fn menubar_sub_trigger(label: &str, item: MenuItem) -> Part<MenuSubTriggerProps> {
    let t = theme();
    menu_sub_trigger(
        menu_row_base(&t, item)
            .named("menubar-sub-trigger")
            .gap(0.0)
            .child(label.to_owned())
            .child(icon(Icon::ChevronRight).margin_left(Length::Auto)),
        label,
        item,
        MenuFamily::Menubar,
    )
}

/// A submenu's panel: `min-w-[8rem] overflow-hidden shadow-lg`.
#[must_use]
#[track_caller]
pub fn menubar_sub_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t)
                .named("menubar-sub-content")
                .overflow(Overflow::Hidden)
                .shadow(crate::popover::LIFTED_SHADOW),
            Origin::Anchor,
        ),
        MenuPlace::Beside,
        MenuFamily::Menubar,
    )
}
