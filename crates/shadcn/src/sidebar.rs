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
    Bind, Color, ComponentElement, Div, Element, Key, KeyEvent, Model, Properties, Shadow, Side, Signal, TextField,
    UseModel, component, div, icon, provide, px, text, use_context, use_model,
};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::StateLook;
use crate::input::input;
use crate::part::{Part, PartProps};
use crate::separator::{Orientation, separator};
use crate::skeleton::skeleton;
use crate::theme::{Colors, theme};
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
fn ring_on<S: Styled>(mut el: S, i: Interaction, t: &Colors) -> S {
    let (base, ring) = (el.style_mut().ring, Ring::new(2.0, t.sidebar_ring));
    el.ring(move || if i.focus_visible() { Some(ring) } else { base })
}

/// `hover:bg-sidebar-accent hover:text-sidebar-accent-foreground`, while `on` holds.
fn accent_while<S: Styled>(el: S, on: impl Fn() -> bool + Clone + 'static, t: &Colors) -> S {
    el.background_while(on.clone(), t.sidebar_accent)
        .text_color_while(on, t.sidebar_accent_foreground)
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
    let root = div().named("sidebar-wrapper").display(Display::Flex).size(pct(100.0));
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
                    .when(ctx.variant() == SidebarVariant::Inset, |d| d.background(t.sidebar))
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
    Part::new(
        (),
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .size(pct(100.0)),
        |p| {
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
        },
    )
}

/// The sidebar's boxes around `inner`, the panel holding its parts.
fn frame(sb: &SidebarContext, inner: Div) -> Div {
    let t = theme();
    if sb.collapsible() == SidebarCollapsible::None {
        return inner
            .named("sidebar")
            .background(t.sidebar)
            .text_color(t.sidebar_foreground)
            .width(SIDEBAR_WIDTH)
            .height(pct(100.0))
            .flex_shrink(0.0);
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
        .position(Position::Relative)
        .background(t.sidebar)
        .when(sb.variant() == SidebarVariant::Floating, |d| {
            d.border(1.0).border_color(t.sidebar_border).radius(10.0).shadow([
                Shadow {
                    x: 0.0,
                    y: 1.0,
                    blur: 3.0,
                    spread: 0.0,
                    color: Color::BLACK.opacity(0.1),
                },
                Shadow {
                    x: 0.0,
                    y: 1.0,
                    blur: 2.0,
                    spread: -1.0,
                    color: Color::BLACK.opacity(0.1),
                },
            ])
        });
    let container = div()
        .named("sidebar-container")
        .transition(Properties::INSET | Properties::WIDTH)
        .duration_ms(200.0)
        .ease(Easing::Linear)
        .position(Position::Absolute)
        .top(0.0)
        .bottom(0.0)
        .width(width)
        .display(Display::Flex)
        .when(left, |d| d.left(px(offset)))
        .when(!left, |d| d.right(px(offset)))
        .when(padded, |d| d.padding(8.0))
        .when(!padded && left, |d| d.border_right(1.0).border_color(t.border))
        .when(!padded && !left, |d| d.border_left(1.0).border_color(t.border))
        .child(panel);
    // The gap holds the sidebar's place in the row; the container is drawn over it.
    div()
        .named("sidebar")
        .transition(Properties::WIDTH)
        .duration_ms(200.0)
        .ease(Easing::Linear)
        .position(Position::Relative)
        .height(pct(100.0))
        .width(gap)
        .flex_shrink(0.0)
        .text_color(t.sidebar_foreground)
        .child(container)
}

/// A button that opens and closes the sidebar.
#[must_use]
pub fn sidebar_trigger() -> Part<()> {
    let root = button(ButtonVariant::Ghost, ButtonSize::Icon)
        .named("sidebar-trigger")
        .size(28.0)
        .child(icon(gui::Icon::PanelLeft))
        .into_div();
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
        let i = Interaction::new();
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
                .position(Position::Absolute)
                .top(0.0)
                .bottom(0.0)
                .width(2.0)
                .left(Length::Fraction(if offcanvas { 1.0 } else { 0.5 }))
                .background(move || i.hovered().then_some(t.sidebar_border));
            div()
                .named("sidebar-rail")
                .interaction(i)
                .transition(Properties::ALL)
                .ease(Easing::Linear)
                .position(Position::Absolute)
                .top(0.0)
                .bottom(0.0)
                .width(16.0)
                .display(Display::Flex)
                .cursor(gui::Cursor::ResizeColumn)
                .when(side == SidebarSide::Left, |d| d.right(reach))
                .when(side == SidebarSide::Right, |d| d.left(reach))
                .when(offcanvas, |d| d.background_while(move || i.hovered(), t.sidebar))
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
        .background(t.background)
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .flex(Flex::grow(1.0))
        .min_width(0.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match &ctx {
                    Some(sb) if sb.variant() == SidebarVariant::Inset => {
                        let collapsed = !sb.is_open();
                        root.margin(8.0)
                            .margin_left(if collapsed { 8.0 } else { 0.0 })
                            .radius(14.0)
                            .shadow([
                                Shadow {
                                    x: 0.0,
                                    y: 1.0,
                                    blur: 3.0,
                                    spread: 0.0,
                                    color: Color::BLACK.opacity(0.1),
                                },
                                Shadow {
                                    x: 0.0,
                                    y: 1.0,
                                    blur: 2.0,
                                    spread: -1.0,
                                    color: Color::BLACK.opacity(0.1),
                                },
                            ])
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
    div()
        .named("sidebar-header")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .padding(8.0)
}

#[must_use]
pub fn sidebar_footer() -> Div {
    div()
        .named("sidebar-footer")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .padding(8.0)
}

/// The scrolling middle of the sidebar, between header and footer.
#[must_use]
pub fn sidebar_content() -> Part<()> {
    let root = div()
        .named("sidebar-content")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .min_height(0.0)
        .flex(Flex::grow(1.0))
        .gap(8.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                let d = props.with(|p| p.slot.0.clone());
                if icon_collapsed(ctx.as_ref()) {
                    d.overflow(Overflow::Hidden)
                } else {
                    d.overflow_y(Overflow::Scroll).overflow_x(Overflow::Hidden)
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
        .background(t.sidebar_border)
        .margin_x(8.0)
        .width(Length::Auto)
}

/// A search field sized for the sidebar.
#[must_use]
pub fn sidebar_input(field: &TextField) -> crate::input::Input {
    let t = theme();
    input()
        .field(*field)
        .named("sidebar-input")
        .background(t.background)
        .height(32.0)
        .width(pct(100.0))
        .shadow(Shadows::new())
}

#[must_use]
pub fn sidebar_group() -> Div {
    div()
        .named("sidebar-group")
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .padding(8.0)
}

/// A group's heading; slides up out of sight when collapsed to icons.
#[must_use]
pub fn sidebar_group_label() -> Part<()> {
    let t = theme();
    let root = div()
        .named("sidebar-group-label")
        .transition(Properties::MARGIN | Properties::OPACITY)
        .duration_ms(200.0)
        .ease(Easing::Linear)
        .text_color(t.sidebar_foreground.opacity(0.7))
        .display(Display::Flex)
        .height(32.0)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .radius(8.0)
        .padding_x(8.0)
        .text_size(12.0)
        .line_height(16.0)
        .font_weight(500)
        .with_interaction(|d, i| ring_on(d, i, &t));
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = SidebarContext::find();
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .when(icon_collapsed(ctx.as_ref()), |d| {
                        d.margin_top(-32.0).opacity(0.0).pass_through()
                    })
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
                    .when(icon_collapsed(ctx.as_ref()), |e| e.display(Display::None))
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
            .transition(Properties::TRANSFORM)
            .text_color(t.sidebar_foreground)
            .position(Position::Absolute)
            .top(14.0)
            .right(12.0)
            .display(Display::Flex)
            .size(20.0)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .radius(8.0)
            .focusable()
            .with_interaction(|d, i| ring_on(accent_while(d, move || i.hovered(), &t), i, &t)),
    )
}

#[must_use]
pub fn sidebar_group_content() -> Div {
    div()
        .named("sidebar-group-content")
        .width(pct(100.0))
        .text_size(14.0)
        .line_height(20.0)
}

#[must_use]
pub fn sidebar_menu() -> Div {
    div()
        .named("sidebar-menu")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .gap(4.0)
}

/// What a menu item's action and badge read: whether the pointer is in it.
#[derive(Clone, Copy)]
struct ItemContext {
    /// The item's: its action and badge show while the pointer is on it.
    item: Interaction,
}

/// One entry of a menu: its button, and an action or a badge, go inside.
/// The action and badge follow the pointer in it.
#[must_use]
pub fn sidebar_menu_item() -> Part<()> {
    Part::new((), div().named("sidebar-menu-item").position(Position::Relative), |p| {
        component(p, |cx| {
            let props = cx.props();
            let item = Interaction::new();
            provide(ItemContext { item });
            move || props.with(|p| p.slot.0.clone()).interaction(item).into_element()
        })
    })
}

fn item_interaction() -> Option<Interaction> {
    use_context::<ItemContext>().map(|i| i.item)
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
    /// How far below the item's top its action and badge sit, in pixels (`top-1` … `top-2.5`).
    fn inner_top(self) -> f32 {
        match self {
            Self::Sm => 4.0,
            Self::Default => 6.0,
            Self::Lg => 10.0,
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
/// the label `.truncate(true).nowrap(true).overflow_x(Overflow::Hidden).min_width(0.0)`), and `.padding_right(32.0)` when the item has an action.
#[must_use]
pub fn sidebar_menu_button(props: SidebarMenuButtonProps) -> Part<SidebarMenuButtonProps> {
    let t = theme();
    let base = div()
        .named("sidebar-menu-button")
        .transition(Properties::WIDTH | Properties::HEIGHT | Properties::PADDING)
        .display(Display::Flex)
        .width(pct(100.0))
        .align_items(Align::Center)
        .gap(8.0)
        .overflow(Overflow::Hidden)
        .radius(8.0)
        .padding(8.0)
        .text_align(TextAlign::Left)
        .text_size(14.0)
        .line_height(20.0)
        .focusable()
        .when(props.is_active, |d| {
            d.background(t.sidebar_accent)
                .font_weight(500)
                .text_color(t.sidebar_accent_foreground)
        });
    let i = Interaction::new();
    let base = base.interaction(i);
    let styled = match props.variant {
        SidebarMenuButtonVariant::Default => base,
        SidebarMenuButtonVariant::Outline => {
            base.when(!props.is_active, |d| d.background(t.background))
                .shadow(move || {
                    outline(if i.hovered() {
                        t.sidebar_accent
                    } else {
                        t.sidebar_border
                    })
                })
        }
    };
    let styled = ring_on(accent_while(styled, move || i.hovered() || i.pressed(), &t), i, &t).dim_disabled(i, 0.5);
    let sized = match props.size {
        SidebarMenuButtonSize::Default => styled.height(32.0).text_size(14.0).line_height(20.0),
        SidebarMenuButtonSize::Sm => styled.height(28.0).text_size(12.0).line_height(16.0),
        SidebarMenuButtonSize::Lg => styled.height(48.0).text_size(14.0).line_height(20.0),
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
                    d.size(32.0)
                        .when(size == SidebarMenuButtonSize::Lg, |d| d.padding(0.0))
                        .when(size != SidebarMenuButtonSize::Lg, |d| d.padding(8.0))
                });
                match tip {
                    // The tooltip shows only while collapsed.
                    Some(label) => tooltip()
                        .width(pct(100.0))
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
        .transition(Properties::TRANSFORM)
        .position(Position::Absolute)
        .top(size.inner_top())
        .right(4.0)
        .display(Display::Flex)
        .size(20.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(8.0)
        .focusable();
    let i = Interaction::new();
    let root = ring_on(
        root.interaction(i)
            .background_while(move || i.hovered(), t.sidebar_accent),
        i,
        &t,
    );
    aside(SidebarMenuAside { show_on_hover }, root)
}

/// A count at the right of its menu item, whose button has `size`.
#[must_use]
pub fn sidebar_menu_badge(size: SidebarMenuButtonSize) -> Part<SidebarMenuAside> {
    let root = div()
        .named("sidebar-menu-badge")
        .pass_through()
        .position(Position::Absolute)
        .top(size.inner_top())
        .right(4.0)
        .display(Display::Flex)
        .height(20.0)
        .min_width(20.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(8.0)
        .padding_x(4.0)
        .text_size(12.0)
        .line_height(16.0)
        .font_weight(500);
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
            let item = item_interaction();
            move || {
                let t = theme();
                let (show_on_hover, root) = props.with(|p| (p.props.show_on_hover, p.slot.0.clone()));
                let on_item = move || item.is_some_and(Interaction::hovered);
                // Its own hover (an action's `hover:text-sidebar-accent-foreground`) or the item's.
                root.with_interaction(|d, own| {
                    d.text_color(move || {
                        if own.hovered() || on_item() {
                            t.sidebar_accent_foreground
                        } else {
                            t.sidebar_foreground
                        }
                    })
                })
                .when(show_on_hover, |d| d.opacity(move || if on_item() { 1.0 } else { 0.0 }))
                .when(icon_collapsed(ctx.as_ref()), |e| e.display(Display::None))
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
        .display(Display::Flex)
        .height(32.0)
        .align_items(Align::Center)
        .gap(8.0)
        .radius(8.0)
        .padding_x(8.0)
        .when(show_icon, |d| d.child(skeleton().size(16.0)))
        .child(
            skeleton()
                .height(16.0)
                .width(Length::Fraction(skeleton_width(id)))
                .min_width(0.0),
        )
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
            .border_left(1.0)
            .border_color(t.sidebar_border)
            .margin_x(14.0)
            .position(Position::Relative)
            .left(px(1.0))
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .min_width(0.0)
            .gap(4.0)
            .padding_x(10.0)
            .padding_y(2.0),
    )
}

#[must_use]
pub fn sidebar_menu_sub_item() -> Div {
    div().named("sidebar-menu-sub-item").position(Position::Relative)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarMenuSubButtonSize {
    Sm,
    #[default]
    Md,
}

/// A link in a sub menu; add its label with `.child` (give it `.truncate(true).nowrap(true).overflow_x(Overflow::Hidden).min_width(0.0)`).
/// Its [`sidebar_menu_sub`] hides it when collapsed to icons.
#[must_use]
pub fn sidebar_menu_sub_button(size: SidebarMenuSubButtonSize, is_active: bool) -> Div {
    let t = theme();
    div()
        .named("sidebar-menu-sub-button")
        .text_color(t.sidebar_foreground)
        .display(Display::Flex)
        .height(28.0)
        .min_width(0.0)
        .position(Position::Relative)
        .left(px(-1.0))
        .align_items(Align::Center)
        .gap(8.0)
        .overflow(Overflow::Hidden)
        .radius(8.0)
        .padding_x(8.0)
        .focusable()
        .when(is_active, |d| {
            d.background(t.sidebar_accent).text_color(t.sidebar_accent_foreground)
        })
        .with_interaction(|d, i| {
            ring_on(accent_while(d, move || i.hovered() || i.pressed(), &t), i, &t).dim_disabled(i, 0.5)
        })
        .when(size == SidebarMenuSubButtonSize::Sm, |e| {
            e.text_size(12.0).line_height(16.0)
        })
        .when(size == SidebarMenuSubButtonSize::Md, |e| {
            e.text_size(14.0).line_height(20.0)
        })
}
