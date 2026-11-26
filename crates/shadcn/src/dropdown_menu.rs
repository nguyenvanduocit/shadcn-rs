//! Dropdown menu (`ui/dropdown-menu`), and the menu parts every menu shares.
//!
//! The behaviour is `primitives`' (ADR-0025, group 4): the menu engine
//! ([`primitives::menu::Menu`]) and the dropdown's trigger
//! ([`primitives::dropdown_menu::DropdownMenu`]). This file draws them:
//! each part makes its look once and hands it to the primitive's part
//! function, which adds the behaviour. The parts nest as in Vue:
//! [`dropdown_menu`] keeps whether it is open, its own (`default_open`) or
//! the caller's (`open`, `v-model:open`), and is the anchor its
//! [`dropdown_menu_content`] opens against; the [`dropdown_menu_trigger`]
//! and the items below read it. A [`dropdown_menu_sub`] is a level of its
//! own, holding its sub trigger and sub content.
//!
//! ```ignore
//! dropdown_menu()
//!     .child(dropdown_menu_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(dropdown_menu_content().align(SideAlign::Start).width(224.0)
//!         .child(dropdown_menu_item("Profile", MenuItem::default()).on_select(|_| …).child("Profile")))
//! ```
//!
//! The highlight is focus: a row is lit while its node holds focus
//! ([`gui::Interaction::focused`]), so moving it renders no row. A sub
//! trigger is lit while focused or while its submenu is open (`focus:bg-accent
//! data-open:bg-accent`, in all three menus).

use std::rc::Rc;

use gui::prelude::*;
use gui::{
    Bind, Callback, Dismiss, Div, Element, FocusHandle, Icon, Interaction, Model, Positioning, Side, SideAlign, Text,
    Toggled, UseModel, component, deferred, div, icon, provide, text, use_context, use_model,
};

use gui::style::Origin;

use primitives::context_menu::ContextMenu;
use primitives::dropdown_menu::DropdownMenu;
use primitives::menu::{Menu, MenuOptions};
use primitives::{Availability, Item};

use crate::focus::StateLook;
use crate::motion::popper;
use crate::part::{Part, PartProps, part_root};
use crate::theme::{Colors, theme};

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

    fn availability(self) -> Availability {
        Availability::from_disabled(self.disabled)
    }
}

/// The look every menu row shares: `relative flex cursor-default items-center
/// gap-2 rounded-sm px-2 py-1.5 text-sm select-none data-[disabled]:opacity-50
/// data-[inset]:pl-8`, highlighted as `focus:bg-accent focus:text-accent-foreground`
/// (the destructive variant: `text-destructive focus:bg-destructive/10`, `/20` in dark).
pub(crate) fn menu_row(t: &Colors, item: MenuItem, highlighted: bool) -> Div {
    menu_highlight(menu_row_base(t, item), t, item, highlighted)
}

/// [`menu_row`] not highlighted, for a part to highlight.
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

/// A row's highlight, on top of [`menu_row_base`], decided in render.
pub(crate) fn menu_highlight(d: Div, t: &Colors, item: MenuItem, highlighted: bool) -> Div {
    let destructive = item.variant == MenuItemVariant::Destructive;
    d.when(highlighted && !destructive, |d| {
        d.background(t.accent).text_color(t.accent_foreground)
    })
    .when(highlighted && destructive, |d| {
        d.background(t.destructive.opacity(if t.dark { 0.2 } else { 0.1 }))
    })
}

/// A row's highlight bound to `lit` (read where the node resolves, so a
/// highlight moving renders nothing), on top of [`menu_row_base`].
fn menu_lit(d: Div, t: &Colors, item: MenuItem, lit: impl Fn() -> bool + Clone + 'static) -> Div {
    if item.variant == MenuItemVariant::Destructive {
        d.background_while(lit, t.destructive.opacity(if t.dark { 0.2 } else { 0.1 }))
    } else {
        d.background_while(lit.clone(), t.accent)
            .text_color_while(lit, t.accent_foreground)
    }
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
/// filled circle in the row's text colour, which its highlight turns.
fn menu_dot(t: &Colors, row: Interaction) -> gui::Element {
    let (lit, rest) = (t.accent_foreground, t.popover_foreground);
    div()
        .size(8.0)
        .radius(9999.0)
        .background(move || Some(if row.focused() { lit } else { rest }))
        .into_element()
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

/// A separator between a menu's items: [`menu_separator`], read as one.
pub(crate) fn menu_item_separator(t: &Colors) -> Div {
    primitives::menu::separator(menu_separator(t))
}

/// A group of items: `role=group`, unnamed (it has no label part to point
/// at; reka points at one that may not exist).
pub(crate) fn menu_group(d: Div) -> Div {
    primitives::menu::group(d, None)
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

/// What a menu root is given: whether it is open, and whether it is modal.
#[derive(Clone, PartialEq)]
pub struct MenuRootProps {
    open: Bind<bool>,
    modal: bool,
    /// A context menu's long press (reka's `pressOpenDelay`); none: the
    /// engine's 700 ms.
    press_open_delay: Option<std::time::Duration>,
    /// Which public menu a submenu of it is made as (a report names it).
    family: MenuFamily,
}

impl Default for MenuRootProps {
    fn default() -> Self {
        Self {
            open: Bind::default(),
            modal: MenuOptions::DEFAULT.modal,
            press_open_delay: None,
            family: MenuFamily::Dropdown,
        }
    }
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
    pub fn on_open_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.bind(|b| b.on_change(f)))
    }
}

impl Part<MenuRootProps> {
    /// Modal or not (`:modal`, reka: modal): a modal menu keeps focus in,
    /// and the page below takes no pointer. Read as the root is made.
    #[must_use]
    pub fn modal(self, modal: bool) -> Self {
        self.with_props(|p| p.modal = modal)
    }

    /// How long a touch or pen is held to open a context menu
    /// (`:press-open-delay`, reka: 700 ms). Read as the root is made; a
    /// dropdown menu has no long press.
    #[must_use]
    pub fn press_open_delay(self, delay: std::time::Duration) -> Self {
        self.with_props(|p| p.press_open_delay = Some(delay))
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

    /// The menu's options, as the root is made.
    pub(crate) fn options(&self) -> MenuOptions {
        MenuOptions::DEFAULT.modal(self.modal)
    }

    /// A context menu's long press delay, if the caller set one.
    pub(crate) fn press_open_delay(&self) -> Option<std::time::Duration> {
        self.press_open_delay
    }
}

impl OpenProps for MenuRootProps {
    fn bind(&mut self, f: impl FnOnce(Bind<bool>) -> Bind<bool>) {
        self.open = f(std::mem::take(&mut self.open));
    }
}

/// The nearest menu level, from a part's setup.
///
/// Outside a menu it is a reported error naming the public part of
/// `family` that `kind` says (`"item"`: `dropdown_menu_item`, …) and the
/// root that family needs, and `None`.
pub(crate) fn menu_level<P: 'static>(
    props: gui::Signal<PartProps<P>>,
    family: MenuFamily,
    kind: MenuKind,
) -> Option<Menu> {
    part_root(props, family.part(kind), family.root())
}

/// Which public part of a menu family a shared menu part is made as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Content,
    SubContent,
    Item,
    CheckboxItem,
    RadioItem,
    RadioGroup,
    Sub,
    SubTrigger,
}

/// Which public menu a shared menu part is made as: what a report of a
/// missing root names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuFamily {
    Dropdown,
    Context,
    Menubar,
}

impl MenuFamily {
    /// The root its parts need.
    fn root(self) -> &'static str {
        match self {
            Self::Dropdown => "dropdown_menu",
            Self::Context => "context_menu",
            Self::Menubar => "menubar_menu",
        }
    }

    /// The public part `kind` of this family, as a static name (the
    /// report's set is bounded by the call sites).
    pub(crate) fn part(self, kind: MenuKind) -> &'static str {
        use MenuKind as K;
        match (self, kind) {
            (Self::Dropdown, K::Content) => "dropdown_menu_content",
            (Self::Dropdown, K::SubContent) => "dropdown_menu_sub_content",
            (Self::Dropdown, K::Item) => "dropdown_menu_item",
            (Self::Dropdown, K::CheckboxItem) => "dropdown_menu_checkbox_item",
            (Self::Dropdown, K::RadioItem) => "dropdown_menu_radio_item",
            (Self::Dropdown, K::RadioGroup) => "dropdown_menu_radio_group",
            (Self::Dropdown, K::Sub) => "dropdown_menu_sub",
            (Self::Dropdown, K::SubTrigger) => "dropdown_menu_sub_trigger",
            (Self::Context, K::Content) => "context_menu_content",
            (Self::Context, K::SubContent) => "context_menu_sub_content",
            (Self::Context, K::Item) => "context_menu_item",
            (Self::Context, K::CheckboxItem) => "context_menu_checkbox_item",
            (Self::Context, K::RadioItem) => "context_menu_radio_item",
            (Self::Context, K::RadioGroup) => "context_menu_radio_group",
            (Self::Context, K::Sub) => "context_menu_sub",
            (Self::Context, K::SubTrigger) => "context_menu_sub_trigger",
            (Self::Menubar, K::Content) => "menubar_content",
            (Self::Menubar, K::SubContent) => "menubar_sub_content",
            (Self::Menubar, K::Item) => "menubar_item",
            (Self::Menubar, K::CheckboxItem) => "menubar_checkbox_item",
            (Self::Menubar, K::RadioItem) => "menubar_radio_item",
            (Self::Menubar, K::RadioGroup) => "menubar_radio_group",
            (Self::Menubar, K::Sub) => "menubar_sub",
            (Self::Menubar, K::SubTrigger) => "menubar_sub_trigger",
        }
    }
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
    /// Where its context menu says: at the pointer that opened it.
    Pointer,
    /// Beside its sub trigger, on the inline end (a submenu).
    Beside,
}

/// What a menu's content is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuContentProps {
    place: MenuPlace,
    family: MenuFamily,
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
/// floating at `place`, the engine's content (keys, pointer, layer).
#[track_caller]
pub(crate) fn menu_content(root: Div, place: MenuPlace, family: MenuFamily) -> Part<MenuContentProps> {
    Part::new(MenuContentProps { place, family }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let (place, family) = props.with_untracked(|p| (p.props.place, p.props.family));
            let kind = if place == MenuPlace::Beside {
                MenuKind::SubContent
            } else {
                MenuKind::Content
            };
            let level = menu_level(props, family, kind);
            let context = use_context::<ContextMenu>();
            move || {
                let Some(menu) = level else {
                    return Element::Empty;
                };
                if !menu.is_open() {
                    return Element::Empty;
                }
                let (place, root) = props.with(|p| (p.props.place, p.slot.0.clone()));
                let positioning = match place {
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
                    MenuPlace::Pointer => context.map_or(Positioning::At { x: 0.0, y: 0.0 }, ContextMenu::positioning),
                    MenuPlace::Beside => Positioning::Anchored {
                        side: menu.sub_side(),
                        align: SideAlign::Start,
                        offset: 0.0,
                        match_width: false,
                        fit_height: false,
                    },
                };
                deferred(positioning, menu.content(root))
                    .placement(menu.placed())
                    .into_element()
            }
        })
    })
}

/// A pick of a menu item (reka's `select` event), handed to its
/// `on_select`: the menu closes after it unless the handler keeps it open
/// ([`Self::keep_open`], reka's `event.preventDefault()`).
///
/// Made by the pick, in the world that picks: a check's pick keeps its own.
#[derive(Clone, Debug, Default)]
pub struct SelectEvent {
    keep: Rc<gui::engine::WorldCell<bool>>,
}

impl SelectEvent {
    /// Leave the menu open after this pick.
    pub fn keep_open(&self) {
        self.keep.set(true);
    }

    /// Whether a handler kept the menu open.
    #[must_use]
    pub fn kept_open(&self) -> bool {
        self.keep.get()
    }

    /// Hand a pick to `on_select`; what the menu does then.
    fn pick(on_select: &Callback<Self>) -> Dismiss {
        let event = Self::default();
        on_select.call(event.clone());
        if event.kept_open() {
            Dismiss::Veto
        } else {
            Dismiss::Close
        }
    }
}

/// What an item is given: its id (what typeahead matches), its look, and
/// what picking it runs.
#[derive(Clone)]
pub struct MenuItemProps {
    id: String,
    item: MenuItem,
    on_select: Option<Callback<SelectEvent>>,
    family: MenuFamily,
}

impl Part<MenuItemProps> {
    /// Run `f` when the item is picked (`@select`); the menu then closes,
    /// unless `f` keeps it open ([`SelectEvent::keep_open`]).
    #[must_use]
    pub fn on_select(self, f: impl Into<Callback<SelectEvent>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_select = Some(f))
    }
}

/// An item on `root`, highlighted by the pointer and the keys.
#[track_caller]
pub(crate) fn menu_item(root: Div, id: &str, item: MenuItem, family: MenuFamily) -> Part<MenuItemProps> {
    let props = MenuItemProps {
        id: id.to_owned(),
        item,
        on_select: None,
        family,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level(props, props.with_untracked(|p| p.props.family), MenuKind::Item);
            let focus = FocusHandle::new();
            // The `on_select` passed last, when it is picked.
            let on_select = cx.event_opt(|p| p.props.on_select.as_ref());
            move || {
                let (id, item, root) = props.with(|p| (p.props.id.clone(), p.props.item, p.slot.0.clone()));
                let on_select = on_select.clone();
                let Some(menu) = level else {
                    return root.into_element();
                };
                let t = theme();
                let d = root.with_interaction(|d, i| menu_lit(d, &t, item, move || i.focused()));
                let entry = Item::new((), focus).label(id).availability(item.availability());
                menu.item(d, entry, move |()| SelectEvent::pick(&on_select))
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
    on_select: Option<Callback<SelectEvent>>,
    family: MenuFamily,
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
    pub fn on_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.checked = p.checked.clone().on_change(f))
    }

    /// Run `f` when the item is picked (`@select`), after it turns; the
    /// menu then closes, unless `f` keeps it open ([`SelectEvent::keep_open`]).
    #[must_use]
    pub fn on_select(self, f: impl Into<Callback<SelectEvent>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_select = Some(f))
    }
}

/// A checkbox item on `root` (its label already in it); picking it asks for
/// the other value and closes the menu, unless its `on_select` keeps it open.
#[track_caller]
pub(crate) fn menu_checkbox_item(
    root: Div,
    label: &str,
    disabled: bool,
    family: MenuFamily,
) -> Part<MenuCheckboxItemProps> {
    let props = MenuCheckboxItemProps {
        label: label.to_owned(),
        disabled,
        checked: Bind::default(),
        on_select: None,
        family,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let level = menu_level(props, props.with_untracked(|p| p.props.family), MenuKind::CheckboxItem);
            let checked = use_model(props, |p| &p.props.checked);
            let focus = FocusHandle::new();
            let on_select = cx.event_opt(|p| p.props.on_select.as_ref());
            move || {
                let on_select = on_select.clone();
                let (label, disabled, root) =
                    props.with(|p| (p.props.label.clone(), p.props.disabled, p.slot.0.clone()));
                let Some(menu) = level else {
                    return root.into_element();
                };
                let item = MenuItem {
                    disabled,
                    ..MenuItem::default()
                };
                let state = if checked.get() { Toggled::True } else { Toggled::False };
                let t = theme();
                let d = root
                    .with_interaction(|d, i| menu_lit(d, &t, item, move || i.focused()))
                    .child(menu_indicator(
                        primitives::menu::indicator_shown(state, false).then(menu_check),
                    ));
                let entry = Item::new((), focus).label(label).availability(item.availability());
                menu.checkbox_item(d, entry, state, move |next| {
                    checked.set(next);
                    SelectEvent::pick(&on_select)
                })
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
    pub fn on_change(self, f: impl Into<gui::Callback<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

/// What a radio group's items read: its value.
#[derive(Clone, Copy)]
struct MenuRadio(UseModel<PartProps<MenuRadioGroupProps>, String>);

/// A radio group on `root` (`role=group`); its radio items set its value.
pub(crate) fn menu_radio_group(root: Div) -> Part<MenuRadioGroupProps> {
    Part::new(MenuRadioGroupProps::default(), menu_group(root), |p| {
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
    on_select: Option<Callback<SelectEvent>>,
    family: MenuFamily,
}

impl Part<MenuRadioItemProps> {
    /// Run `f` when the item is picked (`@select`), after its group takes
    /// its value; the menu then closes, unless `f` keeps it open
    /// ([`SelectEvent::keep_open`]).
    #[must_use]
    pub fn on_select(self, f: impl Into<Callback<SelectEvent>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_select = Some(f))
    }
}

/// A radio item on `root` (its label already in it), setting its group to `value`.
#[track_caller]
pub(crate) fn menu_radio_item(
    root: Div,
    label: &str,
    value: &str,
    disabled: bool,
    family: MenuFamily,
) -> Part<MenuRadioItemProps> {
    let props = MenuRadioItemProps {
        label: label.to_owned(),
        value: value.to_owned(),
        disabled,
        on_select: None,
        family,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let family = props.with_untracked(|p| p.props.family);
            let level = menu_level(props, family, MenuKind::RadioItem);
            let group = part_root::<_, MenuRadio>(
                props,
                family.part(MenuKind::RadioItem),
                family.part(MenuKind::RadioGroup),
            );
            let focus = FocusHandle::new();
            let on_select = cx.event_opt(|p| p.props.on_select.as_ref());
            move || {
                let on_select = on_select.clone();
                let (label, value, disabled, root) = props.with(|p| {
                    (
                        p.props.label.clone(),
                        p.props.value.clone(),
                        p.props.disabled,
                        p.slot.0.clone(),
                    )
                });
                let (Some(menu), Some(group)) = (level, group) else {
                    return root.into_element();
                };
                let t = theme();
                let item = MenuItem {
                    disabled,
                    ..MenuItem::default()
                };
                let chosen = group.0.with(|v| *v == value);
                let shown =
                    primitives::menu::indicator_shown(if chosen { Toggled::True } else { Toggled::False }, false);
                let d = root.with_interaction(|d, i| {
                    menu_lit(d, &t, item, move || i.focused()).child(menu_indicator(shown.then(|| menu_dot(&t, i))))
                });
                let entry = Item::new((), focus).label(label).availability(item.availability());
                menu.radio_item(d, entry, chosen, move |()| {
                    group.0.set(value.clone());
                    SelectEvent::pick(&on_select)
                })
                .into_element()
            }
        })
    })
}

/// A submenu on `root`, a level of its own under the nearest one: it holds
/// its sub trigger and its sub content.
#[track_caller]
pub(crate) fn menu_sub(root: Div, family: MenuFamily) -> Part<MenuRootProps> {
    let props = MenuRootProps {
        family,
        ..MenuRootProps::default()
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            if let Some(parent) = menu_level(props, props.with_untracked(|p| p.props.family), MenuKind::Sub) {
                provide(parent.submenu(use_model(props, |p| p.props.open_bind())));
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
    family: MenuFamily,
}

/// A submenu's row on `root` (label and chevron already in it), in its
/// parent level: lit while focused or open (`focus:bg-accent
/// data-open:bg-accent`).
#[track_caller]
pub(crate) fn menu_sub_trigger(
    root: Div,
    label: &str,
    item: MenuItem,
    family: MenuFamily,
) -> Part<MenuSubTriggerProps> {
    let props = MenuSubTriggerProps {
        label: label.to_owned(),
        item,
        family,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let sub = menu_level(props, props.with_untracked(|p| p.props.family), MenuKind::SubTrigger);
            move || {
                let (label, item, root) = props.with(|p| (p.props.label.clone(), p.props.item, p.slot.0.clone()));
                let Some(sub) = sub else {
                    return root.into_element();
                };
                let t = theme();
                let d = root.with_interaction(|d, i| menu_lit(d, &t, item, move || i.focused() || sub.is_open()));
                sub.sub_trigger(d, label, item.availability()).into_element()
            }
        })
    })
}

// ── The dropdown menu ──

/// The dropdown menu's root (`DropdownMenu`): the anchor its content opens
/// against. Give it a [`dropdown_menu_trigger`] and a [`dropdown_menu_content`].
#[must_use]
pub fn dropdown_menu() -> Part<MenuRootProps> {
    Part::new(
        MenuRootProps::default(),
        div().named("dropdown-menu").display(Display::Flex).inline(true),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let options = props.with_untracked(|p| p.props.options());
                let menu = DropdownMenu::new(use_model(props, |p| p.props.open_bind()), options);
                provide(menu);
                provide(menu.menu());
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

/// Make `trigger` (usually a button) toggle the menu (`DropdownMenuTrigger as-child`).
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
#[track_caller]
pub fn dropdown_menu_trigger(trigger: impl Into<Div>) -> Part<()> {
    Part::new((), trigger.into().named("dropdown-menu-trigger"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let menu = part_root::<_, DropdownMenu>(props, "dropdown_menu_trigger", "dropdown_menu");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match menu {
                    Some(m) => m.trigger(root, Availability::Enabled),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// The floating panel (`DropdownMenuContent`, `side-offset 4`): below the
/// trigger (the other side when only that has room), centred on it unless
/// told otherwise, no taller than the room there. Add groups, items, labels
/// and separators with `.child`.
#[must_use]
#[track_caller]
pub fn dropdown_menu_content() -> Part<MenuContentProps> {
    let t = theme();
    menu_content(
        popper(
            menu_box(&t)
                .named("dropdown-menu-content")
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
        MenuFamily::Dropdown,
    )
}

/// A group of items (`DropdownMenuGroup`, `role=group`).
#[must_use]
pub fn dropdown_menu_group() -> Div {
    menu_group(div().named("dropdown-menu-group"))
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
#[track_caller]
pub fn dropdown_menu_item(id: &str, item: MenuItem) -> Part<MenuItemProps> {
    let t = theme();
    menu_item(
        menu_row_base(&t, item).named("dropdown-menu-item"),
        id,
        item,
        MenuFamily::Dropdown,
    )
}

/// A checkbox item labelled `label`, its own (`default_checked`) or bound to
/// the caller's (`model`, `v-model`); picking it toggles.
#[must_use]
#[track_caller]
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
        MenuFamily::Dropdown,
    )
}

/// A radio item setting its group's value to `value`.
#[must_use]
#[track_caller]
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
        MenuFamily::Dropdown,
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
    menu_item_separator(&theme()).named("dropdown-menu-separator")
}

#[must_use]
pub fn dropdown_menu_shortcut(keys: &str) -> Text {
    menu_shortcut(&theme(), keys).named("dropdown-menu-shortcut")
}

/// A submenu (`DropdownMenuSub`): give it a [`dropdown_menu_sub_trigger`]
/// and a [`dropdown_menu_sub_content`].
#[must_use]
#[track_caller]
pub fn dropdown_menu_sub() -> Part<MenuRootProps> {
    menu_sub(div().named("dropdown-menu-sub"), MenuFamily::Dropdown)
}

/// The row opening its submenu, whose content opens on the inline end. Its
/// chevron is muted, as the item's `[&_svg]` rule makes it.
#[must_use]
#[track_caller]
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
        MenuFamily::Dropdown,
    )
}

/// A submenu's panel: as the content, with `shadow-lg` and `overflow-hidden`.
#[must_use]
#[track_caller]
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
        MenuFamily::Dropdown,
    )
}
