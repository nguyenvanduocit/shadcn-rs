//! Sidebar (`ui/sidebar`): a side panel that collapses off canvas, to icons,
//! or not at all, in three looks (`sidebar`, `floating`, `inset`).
//!
//! [`SidebarState`] is shadcn's `SidebarProvider` context: whether the sidebar
//! is open, its side, variant and collapse mode, and which menu item the
//! pointer is in. The parts whose look follows it (Tailwind's
//! `group-data-[collapsible=icon]:` selectors) take it as an argument.
//! A desktop window has no mobile breakpoint, so the mobile sheet is not
//! ported.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gui::prelude::*;
use gui::{
    Div, ElementKey, Key, KeyEvent, Properties, Shadow, Side, SideAlign, Signal, TextField, TextInput, div, icon,
    key_of, px, relative, text,
};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::input::input;
use crate::separator::{Orientation, separator};
use crate::skeleton::skeleton;
use crate::theme::{Theme, theme};
use crate::tooltip::{TooltipState, tooltip_content_placed, tooltip_trigger};

/// `--sidebar-width`: 16rem.
pub const SIDEBAR_WIDTH: f32 = 256.0;
/// `--sidebar-width-icon`: 3rem.
pub const SIDEBAR_WIDTH_ICON: f32 = 48.0;
/// With ⌘ or Ctrl, toggles the sidebar.
const SHORTCUT: &str = "b";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarSide {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarVariant {
    #[default]
    Sidebar,
    Floating,
    Inset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarCollapsible {
    #[default]
    Offcanvas,
    Icon,
    None,
}

/// What a sidebar shows and how it collapses; create once, outside the render
/// closure, and pass to the parts.
#[derive(Clone)]
pub struct SidebarState {
    /// Expanded (`true`) or collapsed; the `v-model:open` of the provider.
    pub open: Signal<bool>,
    side: SidebarSide,
    variant: SidebarVariant,
    collapsible: SidebarCollapsible,
    /// The menu item under the pointer, for its tooltip and show-on-hover action.
    hovered: Signal<Option<ElementKey>>,
    /// Each menu button's tooltip, made the first time the button is drawn.
    tooltips: Rc<RefCell<HashMap<ElementKey, TooltipState>>>,
}

impl SidebarState {
    /// A left, `sidebar`-variant, off-canvas sidebar, open when `default_open`.
    #[must_use]
    pub fn new(default_open: bool) -> Self {
        Self {
            open: Signal::labeled("sidebar open", default_open),
            side: SidebarSide::default(),
            variant: SidebarVariant::default(),
            collapsible: SidebarCollapsible::default(),
            hovered: Signal::labeled("sidebar hovered item", None),
            tooltips: Rc::default(),
        }
    }

    #[must_use]
    pub fn side(mut self, side: SidebarSide) -> Self {
        self.side = side;
        self
    }

    #[must_use]
    pub fn variant(mut self, variant: SidebarVariant) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn collapsible(mut self, collapsible: SidebarCollapsible) -> Self {
        self.collapsible = collapsible;
        self
    }

    pub fn set_open(&self, open: bool) {
        self.open.set(open);
    }

    pub fn toggle(&self) {
        self.open.update(|o| !o);
    }

    /// Collapsed to its icon rail (`group-data-[collapsible=icon]`).
    #[must_use]
    pub fn icon_collapsed(&self) -> bool {
        self.collapsible == SidebarCollapsible::Icon && !self.open.get()
    }

    /// `floating` and `inset` sit in 8px of padding.
    fn padded(&self) -> bool {
        matches!(self.variant, SidebarVariant::Floating | SidebarVariant::Inset)
    }

    fn is_hovered(&self, id: &str) -> bool {
        self.hovered.get() == Some(key_of(id))
    }

    fn tooltip(&self, id: &str) -> TooltipState {
        self.tooltips.borrow_mut().entry(key_of(id)).or_default().clone()
    }

    fn track_hover(&self, id: &str) -> impl Fn(bool) + 'static {
        let hovered = self.hovered.clone();
        let key = key_of(id);
        move |inside| {
            if inside {
                hovered.set(Some(key));
            } else if hovered.get() == Some(key) {
                hovered.set(None);
            }
        }
    }
}

fn is_shortcut(e: &KeyEvent) -> bool {
    (e.modifiers.command || e.modifiers.control)
        && matches!(&e.key, Key::Character(c) if c.eq_ignore_ascii_case(SHORTCUT))
}

/// `focus-visible:ring-2 ring-sidebar-ring`.
fn ring(s: gui::Style, t: &Theme) -> gui::Style {
    s.ring(2.0, t.sidebar_ring)
}

/// `hover:bg-sidebar-accent hover:text-sidebar-accent-foreground`.
fn accent(s: gui::Style, t: &Theme) -> gui::Style {
    s.bg(t.sidebar_accent).text_color(t.sidebar_accent_foreground)
}

/// A 1px outline drawn as a shadow (`shadow-[0_0_0_1px_<color>]`).
fn outline(color: gui::Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur: 0.0,
        spread: 1.0,
        color,
    }
}

/// The wrapper around a sidebar and its inset: lays them out in a row and
/// toggles the sidebar on ⌘B / Ctrl+B pressed inside it.
#[must_use]
pub fn sidebar_provider(sb: &SidebarState) -> Div {
    let t = theme();
    let state = sb.clone();
    div()
        .named("sidebar-wrapper")
        .flex()
        .size_full()
        .when(sb.variant == SidebarVariant::Inset, |d| d.bg(t.sidebar))
        .on_key_down(move |e| {
            let hit = is_shortcut(e);
            if hit {
                state.toggle();
            }
            hit
        })
}

/// The sidebar holding `children` (header, content, footer, rail).
#[must_use]
pub fn sidebar<E: IntoElement>(sb: &SidebarState, children: impl IntoIterator<Item = E>) -> Div {
    let t = theme();
    if sb.collapsible == SidebarCollapsible::None {
        return div()
            .named("sidebar")
            .bg(t.sidebar)
            .text_color(t.sidebar_foreground)
            .flex_col()
            .w_px(SIDEBAR_WIDTH)
            .h_full()
            .shrink_0()
            .children(children);
    }
    let collapsed = !sb.open.get();
    let padded = sb.padded();
    // Floating and inset keep their 8px padding around the icon rail, plus the border.
    let (gap, width) = match (collapsed, sb.collapsible) {
        (true, SidebarCollapsible::Offcanvas) => (0.0, SIDEBAR_WIDTH),
        (true, SidebarCollapsible::Icon) if padded => (SIDEBAR_WIDTH_ICON + 16.0, SIDEBAR_WIDTH_ICON + 18.0),
        (true, SidebarCollapsible::Icon) => (SIDEBAR_WIDTH_ICON, SIDEBAR_WIDTH_ICON),
        _ => (SIDEBAR_WIDTH, SIDEBAR_WIDTH),
    };
    let offset = if collapsed && sb.collapsible == SidebarCollapsible::Offcanvas {
        -SIDEBAR_WIDTH
    } else {
        0.0
    };
    let left = sb.side == SidebarSide::Left;
    let panel = div()
        .named("sidebar-inner")
        .relative()
        .bg(t.sidebar)
        .flex_col()
        .size_full()
        .when(sb.variant == SidebarVariant::Floating, |d| {
            d.border().border_color(t.sidebar_border).rounded_lg().shadow_sm()
        })
        .children(children);
    let container = div()
        .named("sidebar-container")
        .transition_props(Properties::INSET | Properties::WIDTH)
        .duration_ms(200.0)
        .ease_linear()
        .absolute()
        .top(0.0)
        .bottom(0.0)
        .w_px(width)
        .flex()
        .when(left, |d| d.left_len(px(offset)))
        .when(!left, |d| d.right_len(px(offset)))
        .when(padded, |d| d.p(2.0))
        .when(!padded && left, |d| d.border_r().border_color(t.border))
        .when(!padded && !left, |d| d.border_l().border_color(t.border))
        .child(panel);
    // The gap holds the sidebar's place in the row; the container is drawn over it.
    div()
        .named("sidebar")
        .transition_props(Properties::WIDTH)
        .duration_ms(200.0)
        .ease_linear()
        .relative()
        .h_full()
        .w_px(gap)
        .shrink_0()
        .text_color(t.sidebar_foreground)
        .child(container)
}

/// A button that opens and closes the sidebar.
#[must_use]
pub fn sidebar_trigger(sb: &SidebarState) -> Div {
    let state = sb.clone();
    button(ButtonVariant::Ghost, ButtonSize::Icon)
        .named("sidebar-trigger")
        .size(7.0)
        .child(icon(gui::Icon::PanelLeft))
        .on_click(move |_| state.toggle())
}

/// The strip along the sidebar's inner edge: shows a line on hover and
/// toggles the sidebar on click. Put it last among the sidebar's children.
#[must_use]
pub fn sidebar_rail(sb: &SidebarState) -> Div {
    const ID: &str = "sidebar-rail";
    let t = theme();
    let offcanvas = sb.collapsible == SidebarCollapsible::Offcanvas && !sb.open.get();
    let hovered = sb.is_hovered(ID);
    // Centred on the container's edge, which is the panel's edge plus the padding.
    let reach = px(if sb.padded() { -16.0 } else { -8.0 });
    let state = sb.clone();
    let line = div()
        .absolute()
        .top(0.0)
        .bottom(0.0)
        .w(0.5)
        .left_len(relative(if offcanvas { 1.0 } else { 0.5 }))
        .when(hovered, |d| d.bg(t.sidebar_border));
    div()
        .named("sidebar-rail")
        .transition_all()
        .ease_linear()
        .absolute()
        .top(0.0)
        .bottom(0.0)
        .w(4.0)
        .flex()
        .cursor(gui::Cursor::ResizeColumn)
        .when(sb.side == SidebarSide::Left, |d| d.right_len(reach))
        .when(sb.side == SidebarSide::Right, |d| d.left_len(reach))
        .when(offcanvas, |d| d.hover(move |s| s.bg(t.sidebar)))
        .on_hover(sb.track_hover(ID))
        .on_click(move |_| state.toggle())
        .child(line)
}

/// The main area beside the sidebar; inset in a rounded card for the `inset` variant.
#[must_use]
pub fn sidebar_inset(sb: &SidebarState) -> Div {
    let t = theme();
    let collapsed = !sb.open.get();
    div()
        .named("sidebar-inset")
        .bg(t.background)
        .relative()
        .flex_col()
        .w_full()
        .flex_1()
        .min_w(0.0)
        .when(sb.variant == SidebarVariant::Inset, |d| {
            d.m(2.0).ml(if collapsed { 2.0 } else { 0.0 }).rounded_xl().shadow_sm()
        })
}

#[must_use]
pub fn sidebar_header() -> Div {
    div().named("sidebar-header").flex_col().gap(2.0).p(2.0)
}

#[must_use]
pub fn sidebar_footer() -> Div {
    div().named("sidebar-footer").flex_col().gap(2.0).p(2.0)
}

/// The scrolling middle of the sidebar, between header and footer.
#[must_use]
pub fn sidebar_content(sb: &SidebarState) -> Div {
    let d = div().named("sidebar-content").flex_col().min_h(0.0).flex_1().gap(2.0);
    if sb.icon_collapsed() {
        d.overflow_hidden()
    } else {
        d.overflow_y_scroll()
    }
}

#[must_use]
pub fn sidebar_separator() -> Div {
    let t = theme();
    separator(Orientation::Horizontal)
        .named("sidebar-separator")
        .bg(t.sidebar_border)
        .mx(2.0)
        .w_auto()
}

/// A search field sized for the sidebar.
#[must_use]
pub fn sidebar_input(field: &TextField) -> TextInput {
    let t = theme();
    input(field)
        .named("sidebar-input")
        .bg(t.background)
        .h(8.0)
        .w_full()
        .shadow_none()
}

#[must_use]
pub fn sidebar_group() -> Div {
    div()
        .named("sidebar-group")
        .relative()
        .flex_col()
        .w_full()
        .min_w(0.0)
        .p(2.0)
}

/// A group's heading; slides up out of sight when collapsed to icons.
#[must_use]
pub fn sidebar_group_label(sb: &SidebarState) -> Div {
    let t = theme();
    div()
        .named("sidebar-group-label")
        .transition_props(Properties::MARGIN | Properties::OPACITY)
        .duration_ms(200.0)
        .ease_linear()
        .text_color(t.sidebar_foreground.opacity(0.7))
        .flex()
        .h(8.0)
        .shrink_0()
        .items_center()
        .rounded_md()
        .px(2.0)
        .text_xs()
        .font_medium()
        .focus_visible(move |s| ring(s, &t))
        .when(sb.icon_collapsed(), |d| d.mt(-8.0).opacity(0.0).pass_through())
}

/// A small icon button at the right of a group's label; add its icon with `.child`.
#[must_use]
pub fn sidebar_group_action(sb: &SidebarState) -> Div {
    let t = theme();
    div()
        .named("sidebar-group-action")
        .transition_transform()
        .text_color(t.sidebar_foreground)
        .absolute()
        .top(3.5)
        .right(3.0)
        .flex()
        .size(5.0)
        .items_center()
        .justify_center()
        .rounded_md()
        .focusable()
        .hover(move |s| accent(s, &t))
        .focus_visible(move |s| ring(s, &t))
        .when(sb.icon_collapsed(), Styled::hidden)
}

#[must_use]
pub fn sidebar_group_content() -> Div {
    div().named("sidebar-group-content").w_full().text_sm()
}

#[must_use]
pub fn sidebar_menu() -> Div {
    div().named("sidebar-menu").flex_col().w_full().min_w(0.0).gap(1.0)
}

/// One entry of a menu. `id` names it among its siblings: its button's
/// tooltip and its show-on-hover action follow the pointer by it.
#[must_use]
pub fn sidebar_menu_item(sb: &SidebarState, id: &str) -> Div {
    div().named("sidebar-menu-item").relative().on_hover(sb.track_hover(id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarMenuButtonVariant {
    #[default]
    Default,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarMenuButtonSize {
    #[default]
    Default,
    Sm,
    Lg,
}

impl SidebarMenuButtonSize {
    /// How far below the item's top its action and badge sit (`top-1` … `top-2.5`).
    fn inner_top(self) -> f32 {
        match self {
            Self::Sm => 1.0,
            Self::Default => 1.5,
            Self::Lg => 2.5,
        }
    }
}

/// The props of [`sidebar_menu_button`] beyond its item.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SidebarMenuButtonProps {
    pub variant: SidebarMenuButtonVariant,
    pub size: SidebarMenuButtonSize,
    pub is_active: bool,
    /// Shown to the right while the sidebar is collapsed and the pointer is on the item.
    pub tooltip: Option<String>,
}

/// The button of the menu item `id`; add its icon and label with `.child`
/// (give the label `.truncate()`), and `.pr(8.0)` when the item has an action.
#[must_use]
pub fn sidebar_menu_button(sb: &SidebarState, id: &str, props: SidebarMenuButtonProps) -> Div {
    let t = theme();
    let icons = sb.icon_collapsed();
    let base = div()
        .named("sidebar-menu-button")
        .transition_props(Properties::WIDTH | Properties::HEIGHT | Properties::PADDING)
        .flex()
        .w_full()
        .items_center()
        .gap(2.0)
        .overflow_hidden()
        .rounded_md()
        .p(2.0)
        .text_left()
        .text_sm()
        .focusable()
        .focus_visible(move |s| ring(s, &t))
        .active(move |s| accent(s, &t))
        .disabled_style(|s| s.opacity(0.5))
        .when(props.is_active, |d| {
            d.bg(t.sidebar_accent)
                .font_medium()
                .text_color(t.sidebar_accent_foreground)
        });
    let styled = match props.variant {
        SidebarMenuButtonVariant::Default => base.hover(move |s| accent(s, &t)),
        SidebarMenuButtonVariant::Outline => base
            .when(!props.is_active, |d| d.bg(t.background))
            .shadow(outline(t.sidebar_border))
            .hover(move |s| accent(s, &t).shadow_none().shadow(outline(t.sidebar_accent))),
    };
    let sized = match props.size {
        SidebarMenuButtonSize::Default => styled.h(8.0).text_sm(),
        SidebarMenuButtonSize::Sm => styled.h(7.0).text_xs(),
        SidebarMenuButtonSize::Lg => styled.h(12.0).text_sm().when(icons, |d| d.p(0.0)),
    };
    let collapsed = !sb.open.get();
    let sized = sized.when(icons, |d| {
        d.size(8.0).when(props.size != SidebarMenuButtonSize::Lg, |d| d.p(2.0))
    });
    match props.tooltip {
        Some(label) => {
            let tip = sb.tooltip(id);
            let content = collapsed
                .then(|| tooltip_content_placed(&tip, Side::Right, SideAlign::Center, |c| c.child(text(label))));
            tooltip_trigger(&tip, sized).child(content)
        }
        None => sized,
    }
}

/// A small icon button at the right of the menu item `id`, whose button has
/// `size`; with `show_on_hover` it shows only while the pointer is on the item.
#[must_use]
pub fn sidebar_menu_action(sb: &SidebarState, id: &str, size: SidebarMenuButtonSize, show_on_hover: bool) -> Div {
    let t = theme();
    let hovered = sb.is_hovered(id);
    div()
        .named("sidebar-menu-action")
        .transition_transform()
        .text_color(if hovered {
            t.sidebar_accent_foreground
        } else {
            t.sidebar_foreground
        })
        .absolute()
        .top(size.inner_top())
        .right(1.0)
        .flex()
        .size(5.0)
        .items_center()
        .justify_center()
        .rounded_md()
        .focusable()
        .hover(move |s| accent(s, &t))
        .focus_visible(move |s| ring(s, &t))
        .when(show_on_hover && !hovered, |d| d.opacity(0.0))
        .when(sb.icon_collapsed(), Styled::hidden)
}

/// A count at the right of the menu item `id`, whose button has `size`.
#[must_use]
pub fn sidebar_menu_badge(sb: &SidebarState, id: &str, size: SidebarMenuButtonSize) -> Div {
    let t = theme();
    let hovered = sb.is_hovered(id);
    div()
        .named("sidebar-menu-badge")
        .text_color(if hovered {
            t.sidebar_accent_foreground
        } else {
            t.sidebar_foreground
        })
        .pass_through()
        .absolute()
        .top(size.inner_top())
        .right(1.0)
        .flex()
        .h(5.0)
        .min_w(5.0)
        .items_center()
        .justify_center()
        .rounded_md()
        .px(1.0)
        .text_xs()
        .font_medium()
        .when(sb.icon_collapsed(), Styled::hidden)
}

/// A placeholder menu item while the menu loads; `id` picks its width (50–90%).
#[must_use]
pub fn sidebar_menu_skeleton(id: &str, show_icon: bool) -> Div {
    div()
        .named("sidebar-menu-skeleton")
        .flex()
        .h(8.0)
        .items_center()
        .gap(2.0)
        .rounded_md()
        .px(2.0)
        .when(show_icon, |d| d.child(skeleton().size(4.0)))
        .child(skeleton().h(4.0).w_len(relative(skeleton_width(id))).min_w(0.0))
}

/// shadcn's id hash, giving a stable width between 50% and 90%.
fn skeleton_width(id: &str) -> f32 {
    let mut hash: u32 = 0;
    for c in id.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(c));
    }
    hash = (hash ^ (hash >> 16)).wrapping_mul(0x045D_9F3B);
    hash ^= hash >> 16;
    let step = u8::try_from(hash.checked_rem(40).unwrap_or(0)).unwrap_or(0);
    (50.0 + f32::from(step)) / 100.0
}

/// The nested list under a menu item; hidden when collapsed to icons.
#[must_use]
pub fn sidebar_menu_sub(sb: &SidebarState) -> Div {
    let t = theme();
    div()
        .named("sidebar-menu-sub")
        .border_l()
        .border_color(t.sidebar_border)
        .mx(3.5)
        .relative()
        .left_len(px(1.0))
        .flex_col()
        .min_w(0.0)
        .gap(1.0)
        .px(2.5)
        .py(0.5)
        .when(sb.icon_collapsed(), Styled::hidden)
}

#[must_use]
pub fn sidebar_menu_sub_item() -> Div {
    div().named("sidebar-menu-sub-item").relative()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarMenuSubButtonSize {
    Sm,
    #[default]
    Md,
}

/// A link in a sub menu; add its label with `.child` (give it `.truncate()`).
/// Its [`sidebar_menu_sub`] hides it when collapsed to icons.
#[must_use]
pub fn sidebar_menu_sub_button(size: SidebarMenuSubButtonSize, is_active: bool) -> Div {
    let t = theme();
    div()
        .named("sidebar-menu-sub-button")
        .text_color(t.sidebar_foreground)
        .flex()
        .h(7.0)
        .min_w(0.0)
        .relative()
        .left_len(px(-1.0))
        .items_center()
        .gap(2.0)
        .overflow_hidden()
        .rounded_md()
        .px(2.0)
        .focusable()
        .hover(move |s| accent(s, &t))
        .active(move |s| accent(s, &t))
        .focus_visible(move |s| ring(s, &t))
        .disabled_style(|s| s.opacity(0.5))
        .when(is_active, |d| {
            d.bg(t.sidebar_accent).text_color(t.sidebar_accent_foreground)
        })
        .when(size == SidebarMenuSubButtonSize::Sm, Styled::text_xs)
        .when(size == SidebarMenuSubButtonSize::Md, Styled::text_sm)
}
