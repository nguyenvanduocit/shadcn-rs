//! Sidebar (`ui/sidebar`): a side panel that collapses off canvas, to icons,
//! or not at all, in three looks (`sidebar`, `floating`, `inset`).
//!
//! [`sidebar_provider`] is shadcn's `SidebarProvider`: it keeps whether the
//! sidebar is open, its own (`default_open`) or the caller's (`.model(…)`,
//! `v-model:open`), and here also the sidebar's side, variant and collapse
//! mode, since the inset beside it is styled by them too (Tailwind's
//! `peer-data-[variant=inset]:`). The parts whose look follows it
//! (`group-data-[collapsible=icon]:`) read it; a menu item keeps whether the
//! pointer is in it, for its action and badge. A desktop window has no
//! mobile breakpoint, so the mobile sheet is not ported.

use gui::prelude::*;
use gui::{
    Bind, ComponentElement, Div, Element, Key, KeyEvent, Model, Properties, Shadow, Side, Signal, TextField, UseModel,
    component, div, icon, provide, px, relative, text, use_context, use_model,
};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::input::input;
use crate::part::{Part, PartProps};
use crate::separator::{Orientation, separator};
use crate::skeleton::skeleton;
use crate::theme::{Theme, theme};
use crate::tooltip::{tooltip, tooltip_content, tooltip_trigger};

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

/// What [`sidebar_provider`] is given.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarProps {
    open: Bind<bool>,
    side: SidebarSide,
    variant: SidebarVariant,
    collapsible: SidebarCollapsible,
}

impl Default for SidebarProps {
    fn default() -> Self {
        Self {
            open: Bind::default_value(true),
            side: SidebarSide::default(),
            variant: SidebarVariant::default(),
            collapsible: SidebarCollapsible::default(),
        }
    }
}

/// What the parts of a sidebar read (shadcn's `useSidebar`).
#[derive(Clone)]
struct SidebarContext {
    props: Signal<PartProps<SidebarProps>>,
    open: UseModel<PartProps<SidebarProps>, bool>,
}

impl SidebarContext {
    fn find() -> Option<Self> {
        use_context::<Self>()
    }

    fn side(&self) -> SidebarSide {
        self.props.with(|p| p.props.side)
    }

    fn variant(&self) -> SidebarVariant {
        self.props.with(|p| p.props.variant)
    }

    fn collapsible(&self) -> SidebarCollapsible {
        self.props.with(|p| p.props.collapsible)
    }

    fn is_open(&self) -> bool {
        self.open.get()
    }

    fn toggle(&self) {
        self.open.update(|o| *o = !*o);
    }

    /// Collapsed to its icon rail (`group-data-[collapsible=icon]`).
    fn icon_collapsed(&self) -> bool {
        self.collapsible() == SidebarCollapsible::Icon && !self.is_open()
    }

    /// `floating` and `inset` sit in 8px of padding.
    fn padded(&self) -> bool {
        matches!(self.variant(), SidebarVariant::Floating | SidebarVariant::Inset)
    }
}

/// Whether the sidebar around this instance is collapsed to its icons.
fn icon_collapsed(ctx: Option<&SidebarContext>) -> bool {
    ctx.is_some_and(SidebarContext::icon_collapsed)
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
/// toggles the sidebar on ⌘B / Ctrl+B pressed inside it. Open at first
/// unless told otherwise.
#[must_use]
pub fn sidebar_provider() -> Part<SidebarProps> {
    let root = div().named("sidebar-wrapper").flex().size_full();
    Part::new(SidebarProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext {
                props,
                open: use_model(props, |p| &p.props.open),
            };
            provide(ctx.clone());
            move || {
                let t = theme();
                let keys = ctx.clone();
                props
                    .with(|p| p.slot.0.clone())
                    .when(ctx.variant() == SidebarVariant::Inset, |d| d.bg(t.sidebar))
                    .on_key_down(move |e| {
                        let hit = is_shortcut(e);
                        if hit {
                            keys.toggle();
                        }
                        hit
                    })
                    .into_element()
            }
        })
    })
}

impl Part<SidebarProps> {
    /// Open at first, when no model is given (`default-open`; true unless set).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).with_default(open))
    }

    /// Open or collapsed, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn model(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).with_model(open))
    }

    /// Open or collapsed as the caller shows it (`:open`); a toggle only
    /// calls `on_change`.
    #[must_use]
    pub fn open(self, open: bool) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).with_value(open))
    }

    /// Called with each new open state (`@update:open`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).on_change(f))
    }

    #[must_use]
    pub fn side(self, side: SidebarSide) -> Self {
        self.with_props(|p| p.side = side)
    }

    #[must_use]
    pub fn variant(self, variant: SidebarVariant) -> Self {
        self.with_props(|p| p.variant = variant)
    }

    #[must_use]
    pub fn collapsible(self, collapsible: SidebarCollapsible) -> Self {
        self.with_props(|p| p.collapsible = collapsible)
    }
}

/// The sidebar; its header, content, footer and rail go inside.
#[must_use]
pub fn sidebar() -> Part<()> {
    Part::new((), div().flex_col().size_full(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let inner = props.with(|p| p.slot.0.clone());
                match &ctx {
                    Some(ctx) => frame(ctx, inner),
                    None => inner,
                }
                .into_element()
            }
        })
    })
}

/// The sidebar's boxes around `inner`, the panel holding its parts.
fn frame(sb: &SidebarContext, inner: Div) -> Div {
    let t = theme();
    if sb.collapsible() == SidebarCollapsible::None {
        return inner
            .named("sidebar")
            .bg(t.sidebar)
            .text_color(t.sidebar_foreground)
            .w_px(SIDEBAR_WIDTH)
            .h_full()
            .shrink_0();
    }
    let collapsed = !sb.is_open();
    let padded = sb.padded();
    let collapsible = sb.collapsible();
    // Floating and inset keep their 8px padding around the icon rail, plus the border.
    let (gap, width) = match (collapsed, collapsible) {
        (true, SidebarCollapsible::Offcanvas) => (0.0, SIDEBAR_WIDTH),
        (true, SidebarCollapsible::Icon) if padded => (SIDEBAR_WIDTH_ICON + 16.0, SIDEBAR_WIDTH_ICON + 18.0),
        (true, SidebarCollapsible::Icon) => (SIDEBAR_WIDTH_ICON, SIDEBAR_WIDTH_ICON),
        _ => (SIDEBAR_WIDTH, SIDEBAR_WIDTH),
    };
    let offset = if collapsed && collapsible == SidebarCollapsible::Offcanvas {
        -SIDEBAR_WIDTH
    } else {
        0.0
    };
    let left = sb.side() == SidebarSide::Left;
    let panel = inner
        .named("sidebar-inner")
        .relative()
        .bg(t.sidebar)
        .when(sb.variant() == SidebarVariant::Floating, |d| {
            d.border().border_color(t.sidebar_border).rounded_lg().shadow_sm()
        });
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
pub fn sidebar_trigger() -> Part<()> {
    let root = button(ButtonVariant::Ghost, ButtonSize::Icon)
        .named("sidebar-trigger")
        .size(7.0)
        .child(icon(gui::Icon::PanelLeft));
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match ctx.clone() {
                    Some(ctx) => root.on_click(move |_| ctx.toggle()),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// The strip along the sidebar's inner edge: shows a line on hover and
/// toggles the sidebar on click. Put it last among the sidebar's children.
#[must_use]
pub fn sidebar_rail() -> ComponentElement {
    component((), |_| {
        let ctx = SidebarContext::find();
        let hovered = Signal::labeled("sidebar rail hovered", false);
        move || {
            let Some(sb) = ctx.clone() else {
                return Element::Empty;
            };
            let t = theme();
            let offcanvas = sb.collapsible() == SidebarCollapsible::Offcanvas && !sb.is_open();
            // Centred on the container's edge, which is the panel's edge plus the padding.
            let reach = px(if sb.padded() { -16.0 } else { -8.0 });
            let side = sb.side();
            let line = div()
                .absolute()
                .top(0.0)
                .bottom(0.0)
                .w(0.5)
                .left_len(relative(if offcanvas { 1.0 } else { 0.5 }))
                .when(hovered.get(), |d| d.bg(t.sidebar_border));
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
                .when(side == SidebarSide::Left, |d| d.right_len(reach))
                .when(side == SidebarSide::Right, |d| d.left_len(reach))
                .when(offcanvas, |d| d.hover(move |s| s.bg(t.sidebar)))
                .on_hover(move |inside| hovered.set(inside))
                .on_click(move |_| sb.toggle())
                .child(line)
                .into_element()
        }
    })
}

/// The main area beside the sidebar; inset in a rounded card for the `inset` variant.
#[must_use]
pub fn sidebar_inset() -> Part<()> {
    let t = theme();
    let root = div()
        .named("sidebar-inset")
        .bg(t.background)
        .relative()
        .flex_col()
        .w_full()
        .flex_1()
        .min_w(0.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match &ctx {
                    Some(sb) if sb.variant() == SidebarVariant::Inset => {
                        let collapsed = !sb.is_open();
                        root.m(2.0)
                            .ml(if collapsed { 2.0 } else { 0.0 })
                            .rounded_xl()
                            .shadow_sm()
                    }
                    _ => root,
                }
                .into_element()
            }
        })
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
pub fn sidebar_content() -> Part<()> {
    let root = div().named("sidebar-content").flex_col().min_h(0.0).flex_1().gap(2.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let d = props.with(|p| p.slot.0.clone());
                if icon_collapsed(ctx.as_ref()) {
                    d.overflow_hidden()
                } else {
                    d.overflow_y_scroll()
                }
                .into_element()
            }
        })
    })
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
pub fn sidebar_input(field: &TextField) -> crate::input::Input {
    let t = theme();
    input()
        .field(*field)
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
pub fn sidebar_group_label() -> Part<()> {
    let t = theme();
    let root = div()
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
        .focus_visible(move |s| ring(s, &t));
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .when(icon_collapsed(ctx.as_ref()), |d| d.mt(-8.0).opacity(0.0).pass_through())
                    .into_element()
            }
        })
    })
}

/// Hidden while the sidebar around it is collapsed to icons.
fn hidden_when_icons(root: Div) -> Part<()> {
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .when(icon_collapsed(ctx.as_ref()), Styled::hidden)
                    .into_element()
            }
        })
    })
}

/// A small icon button at the right of a group's label; add its icon with `.child`.
#[must_use]
pub fn sidebar_group_action() -> Part<()> {
    let t = theme();
    hidden_when_icons(
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
            .focus_visible(move |s| ring(s, &t)),
    )
}

#[must_use]
pub fn sidebar_group_content() -> Div {
    div().named("sidebar-group-content").w_full().text_sm()
}

#[must_use]
pub fn sidebar_menu() -> Div {
    div().named("sidebar-menu").flex_col().w_full().min_w(0.0).gap(1.0)
}

/// What a menu item's action and badge read: whether the pointer is in it.
#[derive(Clone, Copy)]
struct ItemContext {
    hovered: Signal<bool>,
}

/// One entry of a menu: its button, and an action or a badge, go inside.
/// The action and badge follow the pointer in it.
#[must_use]
pub fn sidebar_menu_item() -> Part<()> {
    Part::new((), div().named("sidebar-menu-item").relative(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let hovered = Signal::labeled("sidebar item hovered", false);
            provide(ItemContext { hovered });
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .on_hover(move |inside| hovered.set(inside))
                    .into_element()
            }
        })
    })
}

fn item_hovered() -> Option<Signal<bool>> {
    use_context::<ItemContext>().map(|i| i.hovered)
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

/// The button of a menu item; add its icon and label with `.child` (give
/// the label `.truncate()`), and `.pr(8.0)` when the item has an action.
#[must_use]
pub fn sidebar_menu_button(props: SidebarMenuButtonProps) -> Part<SidebarMenuButtonProps> {
    let t = theme();
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
        SidebarMenuButtonSize::Lg => styled.h(12.0).text_sm(),
    };
    Part::new(props, sized, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let (size, tip, root) = props.with(|p| (p.props.size, p.props.tooltip.clone(), p.slot.0.clone()));
                let icons = icon_collapsed(ctx.as_ref());
                let collapsed = ctx.as_ref().is_some_and(|c| !c.is_open());
                let root = root.when(icons, |d| {
                    d.size(8.0)
                        .when(size == SidebarMenuButtonSize::Lg, |d| d.p(0.0))
                        .when(size != SidebarMenuButtonSize::Lg, |d| d.p(2.0))
                });
                match tip {
                    // The tooltip shows only while collapsed.
                    Some(label) => tooltip()
                        .w_full()
                        .child(tooltip_trigger(root))
                        .child(collapsed.then(|| tooltip_content().side(Side::Right).child(text(label))))
                        .into_element(),
                    None => root.into_element(),
                }
            }
        })
    })
}

/// A small icon button at the right of its menu item, whose button has
/// `size`; with `show_on_hover` it shows only while the pointer is on the item.
#[must_use]
pub fn sidebar_menu_action(size: SidebarMenuButtonSize, show_on_hover: bool) -> Part<SidebarMenuAside> {
    let t = theme();
    let root = div()
        .named("sidebar-menu-action")
        .transition_transform()
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
        .focus_visible(move |s| ring(s, &t));
    aside(SidebarMenuAside { show_on_hover }, root)
}

/// A count at the right of its menu item, whose button has `size`.
#[must_use]
pub fn sidebar_menu_badge(size: SidebarMenuButtonSize) -> Part<SidebarMenuAside> {
    let root = div()
        .named("sidebar-menu-badge")
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
        .font_medium();
    aside(SidebarMenuAside { show_on_hover: false }, root)
}

/// What a menu item's action or badge is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidebarMenuAside {
    show_on_hover: bool,
}

/// An action or badge: brighter while the pointer is on its item, hidden
/// when collapsed to icons.
fn aside(props: SidebarMenuAside, root: Div) -> Part<SidebarMenuAside> {
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            let hovered = item_hovered();
            move || {
                let t = theme();
                let (show_on_hover, root) = props.with(|p| (p.props.show_on_hover, p.slot.0.clone()));
                let hovered = hovered.is_some_and(|h| h.get());
                root.text_color(if hovered {
                    t.sidebar_accent_foreground
                } else {
                    t.sidebar_foreground
                })
                .when(show_on_hover && !hovered, |d| d.opacity(0.0))
                .when(icon_collapsed(ctx.as_ref()), Styled::hidden)
                .into_element()
            }
        })
    })
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
pub fn sidebar_menu_sub() -> Part<()> {
    let t = theme();
    hidden_when_icons(
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
            .py(0.5),
    )
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
