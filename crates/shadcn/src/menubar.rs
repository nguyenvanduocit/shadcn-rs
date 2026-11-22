//! Menubar (`ui/menubar`): a row of menus, at most one open; once one is
//! open, pointing at another trigger opens that one, and ArrowLeft or
//! ArrowRight in a menu moves to its neighbour.
//!
//! The parts nest as in Vue: [`menubar`] keeps which menu is open (reka's
//! `modelValue`, the open menu's value or `""`), its own or the caller's
//! (`model`); each [`menubar_menu`] holds a trigger and its content, and is
//! open while the bar's value is its own.

use std::cell::Cell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Bind, Div, Icon, Members, Model, Text, UseModel, component, div, icon, provide, use_context, use_model};

use crate::dropdown_menu::{
    MenuCheckboxItemProps, MenuContentProps, MenuItem, MenuItemProps, MenuNav, MenuPlace, MenuRadioGroupProps,
    MenuRadioItemProps, MenuRootProps, MenuState, MenuSubTriggerProps, OpenModel, menu_box, menu_checkbox_item,
    menu_content, menu_item, menu_level, menu_radio_group, menu_radio_item, menu_row_base, menu_separator,
    menu_shortcut, menu_sub, menu_sub_trigger,
};
use gui::style::Origin;
use gui::{Side, SideAlign};

use crate::motion::{popper, popper_in};
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`menubar`] is given: which menu is open (`""`: none).
#[derive(Clone, PartialEq, Default)]
pub struct MenubarProps {
    value: Bind<String>,
}

impl Part<MenubarProps> {
    /// The open menu's value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// Call `f` with the open menu's value each time it changes.
    #[must_use]
    pub fn on_change(self, f: impl Fn(String) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

type BarValue = UseModel<PartProps<MenubarProps>, String>;

/// What a bar's menus read: which is open, and the menus in tree order.
#[derive(Clone)]
struct MenubarContext {
    value: BarValue,
    menus: Members<(String, MenuState)>,
}

/// A menu of a bar is open while the bar's value is its own.
struct BarOpen {
    bar: BarValue,
    me: String,
}

impl OpenModel for BarOpen {
    fn get(&self) -> bool {
        self.bar.with(|v| *v == self.me)
    }

    fn set(&self, open: bool) {
        if open {
            self.bar.set(self.me.clone());
        } else if self.bar.get_untracked() == self.me {
            self.bar.set(String::new());
        }
    }
}

/// The bar: `bg-background flex h-9 items-center gap-1 rounded-md border p-1
/// shadow-xs`; give it [`menubar_menu`]s.
#[must_use]
pub fn menubar() -> Part<MenubarProps> {
    let t = theme();
    let root = div()
        .named("menubar")
        .role(gui::Role::MenuBar)
        .bg(t.background)
        .flex()
        .h(9.0)
        .items_center()
        .gap(1.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .p(1.0)
        .shadow_xs();
    Part::new(MenubarProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(MenubarContext {
                value: use_model(props, |p| &p.props.value),
                menus: Members::new(),
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
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

thread_local! {
    /// Values for menus not given one (reka's `useId`).
    static NEXT_MENU: Cell<u64> = const { Cell::new(0) };
}

/// One menu of the bar (`MenubarMenu`): the anchor its content opens
/// against. Give it a [`menubar_trigger`] and a [`menubar_content`].
#[must_use]
pub fn menubar_menu() -> Part<MenubarMenuProps> {
    Part::new(MenubarMenuProps::default(), div().named("menubar-menu").flex(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let bar = use_context::<MenubarContext>();
            let me = props.with_untracked(|p| p.props.value.clone()).unwrap_or_else(|| {
                let n = NEXT_MENU.get();
                NEXT_MENU.set(n.wrapping_add(1));
                format!("menubar-menu-{n}")
            });
            let level = match &bar {
                Some(b) => {
                    let open = Rc::new(BarOpen {
                        bar: b.value,
                        me: me.clone(),
                    });
                    let (menus, from) = (b.menus.clone(), me.clone());
                    MenuState::with_open(
                        open,
                        Some(Rc::new(move |forward| move_sideways(&menus, &from, forward))),
                    )
                }
                None => MenuState::new(),
            };
            level.return_focus_on_close();
            provide(level.clone());
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match &bar {
                    Some(b) => root.member(&b.menus, (me.clone(), level.clone())),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// ArrowLeft or ArrowRight past a menu's items: the neighbouring menu opens
/// on its first item (reka's `loop` is off, so the ends stay).
fn move_sideways(menus: &Members<(String, MenuState)>, from: &str, forward: bool) {
    let target = menus.with(|all| {
        let i = all.iter().position(|(v, _)| v == from)?;
        let next = if forward { i.checked_add(1) } else { i.checked_sub(1) }?;
        all.get(next).map(|(_, m)| m.clone())
    });
    if let Some(target) = target {
        target.open_at(MenuNav::First);
    }
}

/// A menu's trigger, `label` (`align start, side-offset 8` for its content).
/// Pointing at it while another menu is open switches to it.
#[must_use]
pub fn menubar_trigger(label: &str) -> Part<()> {
    let t = theme();
    let root = div()
        .named("menubar-trigger")
        .role(gui::Role::MenuItem)
        .flex()
        .items_center()
        .rounded_sm()
        .px(2.0)
        .py(1.0)
        .text_sm()
        .font_medium()
        .focus(move |s| s.bg(t.accent).text_color(t.accent_foreground))
        .child(label.to_owned());
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level();
            let bar = use_context::<MenubarContext>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let Some(level) = level.clone() else {
                    return root.into_element();
                };
                let t = theme();
                let (hover, bar) = (level.clone(), bar.clone());
                level
                    .trigger(root)
                    .when(level.is_open(), |d| d.bg(t.accent).text_color(t.accent_foreground))
                    .on_hover(move |inside| {
                        let any_open = bar.as_ref().is_some_and(|b| !b.value.get_untracked().is_empty());
                        if inside && !gui::untracked(|| hover.is_open()) && any_open {
                            hover.open();
                        }
                    })
                    .into_element()
            }
        })
    })
}

/// The floating panel below its trigger (`align start, side-offset 8`):
/// `bg-popover min-w-[12rem] overflow-hidden rounded-md border p-1
/// shadow-md`; add items with `.child`.
#[must_use]
pub fn menubar_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
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
        MenuPlace::Anchored {
            side: Side::Bottom,
            align: SideAlign::Start,
            offset: 8.0,
            fit_height: false,
        },
    )
}

#[must_use]
pub fn menubar_group() -> Div {
    div().named("menubar-group")
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
pub fn menubar_item(id: &str, item: MenuItem) -> Part<MenuItemProps> {
    let t = theme();
    menu_item(
        menu_row_base(&t, item).named("menubar-item").role(gui::Role::MenuItem),
        id,
        item,
    )
}

/// A checkbox item labelled `label` (`rounded-xs`); picking it toggles.
#[must_use]
pub fn menubar_checkbox_item(label: &str, disabled: bool) -> Part<MenuCheckboxItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_checkbox_item(
        menu_row_base(&t, item)
            .named("menubar-checkbox-item")
            .rounded_xs()
            .pl(8.0)
            .pr(2.0)
            .child(label.to_owned()),
        label,
        disabled,
    )
}

/// A radio item setting its group's value to `value` (`rounded-xs`).
#[must_use]
pub fn menubar_radio_item(label: &str, value: &str, disabled: bool) -> Part<MenuRadioItemProps> {
    let t = theme();
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    menu_radio_item(
        menu_row_base(&t, item)
            .named("menubar-radio-item")
            .rounded_xs()
            .pl(8.0)
            .pr(2.0)
            .child(label.to_owned()),
        label,
        value,
        disabled,
    )
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

/// A submenu (`MenubarSub`): give it a [`menubar_sub_trigger`] and a
/// [`menubar_sub_content`].
#[must_use]
pub fn menubar_sub() -> Part<MenuRootProps> {
    menu_sub(div().named("menubar-sub"))
}

/// The row opening its submenu (`data-[state=open]:bg-accent`, no gap); the
/// content opens to its right.
#[must_use]
pub fn menubar_sub_trigger(label: &str, item: MenuItem) -> Part<MenuSubTriggerProps> {
    let t = theme();
    menu_sub_trigger(
        menu_row_base(&t, item)
            .named("menubar-sub-trigger")
            .gap(0.0)
            .child(label.to_owned())
            .child(icon(Icon::ChevronRight).ml_auto()),
        label,
        item,
        true,
    )
}

/// A submenu's panel: `min-w-[8rem] overflow-hidden shadow-lg`.
#[must_use]
pub fn menubar_sub_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t).named("menubar-sub-content").overflow_hidden().shadow_lg(),
            Origin::Anchor,
        ),
        MenuPlace::Beside,
    )
}
