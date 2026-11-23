//! Navigation menu (`ui/navigation-menu`): a row of triggers, each showing
//! its panel in the shared viewport below the menu (shadcn's default,
//! `viewport`), or below itself with [`NavigationMenuProps`]'s
//! `.viewport(false)` (shadcn's demo). Its behaviour is `primitives`'
//! ([`primitives::navigation_menu::NavigationMenu`]): pointing at a trigger
//! opens its panel after 200 ms (150 ms while one is open, or just after
//! one closed), leaving trigger and panel closes it after 150 ms; a click
//! toggles; the arrows move between triggers and links; Escape, a press
//! outside and focus leaving close.
//!
//! The parts nest as in Vue: [`navigation_menu`] keeps which item is open,
//! its own or the caller's (`model`, `v-model`); each
//! [`navigation_menu_item`] holds its trigger and its content, or a link.

use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Animation, Bind, Dismiss, Div, Element, FocusHandle, Icon, Model, Positioning, Properties, Role, Side, SideAlign,
    component, div, icon, provide, text, use_context, use_model,
};

use primitives::Availability;
use primitives::navigation_menu::{Motion, NavOptions, NavigationMenu, NavigationMenuItem};

use crate::focus::StateLook;
use crate::part::Part;
use crate::theme::{Colors, theme};

/// How far a panel slides in when the pointer moves to it from another
/// trigger (`slide-in-from-right-52`), in pixels.
const SWITCH_SLIDE: f32 = 208.0;

/// What [`navigation_menu`] is given: the open item's value (reka's
/// `modelValue`; `None`: every panel closed).
#[derive(Clone, PartialEq)]
pub struct NavigationMenuProps {
    value: Bind<Option<String>>,
    viewport: bool,
}

impl Default for NavigationMenuProps {
    fn default() -> Self {
        Self {
            value: Bind::default(),
            viewport: true,
        }
    }
}

impl Part<NavigationMenuProps> {
    /// Show the panels in one viewport below the menu (`true`, shadcn's
    /// default) or each below its trigger (`:viewport="false"`).
    #[must_use]
    pub fn viewport(self, viewport: bool) -> Self {
        self.with_props(|p| p.viewport = viewport)
    }

    /// The open item, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Option<String>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// Open first, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = p.value.clone().with_default(Some(value)))
    }

    /// Call `f` with the open item each time it changes.
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<Option<String>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

/// The root: `relative flex max-w-max flex-1 items-center justify-center`,
/// the `navigation` landmark; give it a [`navigation_menu_list`] of
/// [`navigation_menu_item`]s.
#[must_use]
pub fn navigation_menu() -> Part<NavigationMenuProps> {
    let root = div()
        .named("navigation-menu")
        .position(Position::Relative)
        .display(Display::Flex)
        .flex(Flex::grow(1.0))
        .align_items(Align::Center)
        .justify_content(Justify::Center);
    Part::new(NavigationMenuProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let nav = NavigationMenu::new(use_model(props, |p| &p.props.value), NavOptions::DEFAULT);
            provide(nav);
            let panels = Panels(gui::Signal::new(Vec::new()));
            provide(panels);
            move || {
                let (root, viewport) = props.with(|p| (p.slot.0.clone(), p.props.viewport));
                let root = nav.root(root);
                if viewport {
                    root.child(viewport_part(nav, panels)).into_element()
                } else {
                    root.into_element()
                }
            }
        })
    })
}

/// `flex flex-1 items-center justify-center gap-1`, a list.
#[must_use]
pub fn navigation_menu_list() -> Div {
    div()
        .named("navigation-menu-list")
        .role(Role::List)
        .display(Display::Flex)
        .flex(Flex::grow(1.0))
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(4.0)
}

/// What an item and its trigger and content read: the item's value.
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationMenuItemProps {
    value: String,
}

/// An item (`relative`), known by `value`: the anchor its content opens
/// against. Give it a [`navigation_menu_trigger`] and a
/// [`navigation_menu_content`], or a link.
#[must_use]
pub fn navigation_menu_item(value: &str) -> Part<NavigationMenuItemProps> {
    let props = NavigationMenuItemProps {
        value: value.to_owned(),
    };
    Part::new(
        props,
        div().named("navigation-menu-item").position(Position::Relative),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let item = use_context::<NavigationMenu>().map(|nav| {
                    let item = NavigationMenuItem::new(nav, props.with_untracked(|p| p.props.value.clone()));
                    provide(item);
                    item
                });
                move || {
                    let root = props.with(|p| p.slot.0.clone());
                    match item {
                        Some(i) => i.root(root),
                        None => root,
                    }
                    .into_element()
                }
            })
        },
    )
}

/// `navigationMenuTriggerStyle`: `inline-flex h-9 w-max items-center
/// justify-center rounded-md bg-background px-4 py-2 text-sm font-medium
/// hover:bg-accent focus:bg-accent data-[state=open]:bg-accent/50`, for a
/// trigger or a link that looks like one.
#[must_use]
pub fn navigation_menu_trigger_style(open: bool) -> Div {
    trigger_look(div(), open)
}

/// [`navigation_menu_trigger_style`] on `d`.
fn trigger_look(d: Div, open: bool) -> Div {
    let t = theme();
    d.display(Display::Flex)
        .inline(true)
        .height(36.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(8.0)
        .background(if open { t.accent.opacity(0.5) } else { t.background })
        .when(open, |d| d.text_color(t.accent_foreground))
        .padding_x(16.0)
        .padding_y(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .transition(Properties::COLOR | Properties::SHADOW)
        .focusable()
        .with_interaction(|d, i| lit(d, i, &t, Ring::new(3.0, t.ring.opacity(0.5))).dim_disabled(i, 0.5))
}

/// `hover:bg-accent hover:text-accent-foreground focus:bg-accent
/// focus:text-accent-foreground`, and `ring` while focus shows.
fn lit(mut d: Div, i: Interaction, t: &Colors, ring: Ring) -> Div {
    let on = move || i.hovered() || i.focused();
    let base = d.style_mut().ring;
    d.background_while(on, t.accent)
        .text_color_while(on, t.accent_foreground)
        .ring(move || if i.focus_visible() { Some(ring) } else { base })
}

/// The item's trigger, labelled `label`, with a chevron turning over while
/// its panel is open.
#[must_use]
pub fn navigation_menu_trigger(label: &str) -> Part<()> {
    let root = div()
        .named("navigation-menu-trigger")
        // `w-max`: the trigger is as wide as its label on one line.
        .child(text(label.to_owned()).nowrap(true));
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let item = use_context::<NavigationMenuItem>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let Some(item) = item else {
                    return root.into_element();
                };
                let open = item.is_open();
                item.trigger(trigger_look(root, open), Availability::Enabled)
                    .child(
                        icon(Icon::ChevronDown)
                            .position(Position::Relative)
                            .top(gui::px(1.0))
                            .margin_left(4.0)
                            .size(12.0)
                            .transition(Properties::DEFAULT)
                            .duration_ms(300.0)
                            .when(open, |i| i.rotate(180.0)),
                    )
                    .into_element()
            }
        })
    })
}

/// `shadow`: the panel's (and the viewport's) two shadows.
fn popup_shadow() -> [Shadow; 2] {
    [
        Shadow {
            x: 0.0,
            y: 1.0,
            blur: 3.0,
            spread: 0.0,
            color: gui::Color::BLACK.opacity(0.1),
        },
        Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: -1.0,
            color: gui::Color::BLACK.opacity(0.1),
        },
    ]
}

/// The panels a viewport draws: each item's content, by value, as its
/// part last gave it.
type PanelSlot = std::rc::Rc<dyn Fn() -> Div>;

#[derive(Clone, Copy)]
struct Panels(gui::Signal<Vec<(NavigationMenuItem, gui::Registration, PanelSlot)>>);

/// The viewport (`NavigationMenuViewport`): below the menu at its start
/// (`absolute top-full left-0`, `mt-1.5`), `bg-popover
/// text-popover-foreground rounded-lg border shadow overflow-hidden`,
/// opening with `zoom-in-90` and closing with `zoom-out-90` in 100 ms; it
/// draws the open item's panel, and its Tab stops follow that item's
/// trigger.
fn viewport_part(nav: NavigationMenu, panels: Panels) -> Element {
    component((), move |_| {
        nav.use_viewport();
        let t = theme();
        move || {
            let Some(item) = nav.open_item() else {
                return Element::Empty;
            };
            let slot_of = move || {
                panels.0.with(|list| {
                    list.iter()
                        .find(|(i, _, _)| *i == item)
                        .map(|(_, _, slot)| std::rc::Rc::clone(slot))
                })
            };
            if slot_of().is_none() {
                return Element::Empty;
            }
            let key = item.trigger_key();
            let panel = component(key, move |_| {
                provide(Panel(item));
                move || {
                    // Read as it renders, not kept from its setup: a content
                    // replaced in one update gives a new slot to the same
                    // panel.
                    let Some(slot) = slot_of() else {
                        return Element::Empty;
                    };
                    let panel = slot()
                        .animation(opening(item.motion()))
                        .exit_animation(Animation::exit().fade(0.0))
                        .duration_ms(300.0);
                    item.content(panel).into_element()
                }
            })
            .key(key);
            let surface = nav
                .viewport(div().named("navigation-menu-viewport"))
                .position(Position::Relative)
                .overflow(Overflow::Hidden)
                .background(t.popover)
                .text_color(t.popover_foreground)
                .radius(10.0)
                .border(1.0)
                .border_color(t.border)
                .shadow(popup_shadow())
                .animation(Animation::enter().zoom(0.9))
                .exit_animation(Animation::exit().zoom(0.9))
                .duration_ms(100.0)
                .child(panel);
            let placement = Positioning::Anchored {
                side: Side::Bottom,
                align: SideAlign::Start,
                offset: 6.0,
                match_width: false,
                fit_height: false,
            };
            gui::deferred(placement, surface)
                .tab_after(item.trigger_key())
                .into_element()
        }
    })
    .into_element()
}

/// The item's panel (`p-2 pr-2.5`, and with no viewport `bg-popover
/// text-popover-foreground rounded-md border shadow overflow-hidden`),
/// below the item while it is open, or in the menu's viewport; add its
/// links with `.child`.
#[must_use]
pub fn navigation_menu_content() -> Part<()> {
    let root = div().named("navigation-menu-content").padding(8.0).padding_right(10.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let item = use_context::<NavigationMenuItem>();
            if let Some(i) = item {
                provide(Panel(i));
                // A viewport draws it from its slot.
                if let Some(panels) = use_context::<Panels>() {
                    let slot: PanelSlot = std::rc::Rc::new(move || props.with(|p| p.slot.0.clone()));
                    // From its mount to its unmount (a setup, as a render,
                    // writes only what it made); its unmounting takes back
                    // only its own registration: a content that replaced it
                    // registered first.
                    gui::while_mounted(move || {
                        let mine = gui::Registration::new();
                        panels.0.update(|list| list.push((i, mine, slot)));
                        move || {
                            if panels.0.is_alive() {
                                panels.0.update(|list| list.retain(|(_, r, _)| *r != mine));
                            }
                        }
                    });
                }
            }
            let t = theme();
            move || {
                let Some(item) = item else {
                    return Element::Empty;
                };
                if !item.is_open() || item.menu().has_viewport() {
                    return Element::Empty;
                }
                let root = props
                    .with(|p| p.slot.0.clone())
                    .background(t.popover)
                    .text_color(t.popover_foreground)
                    .overflow(Overflow::Hidden)
                    .radius(8.0)
                    .border(1.0)
                    .border_color(t.border)
                    .shadow(popup_shadow());
                let placement = Positioning::Anchored {
                    side: Side::Bottom,
                    align: SideAlign::Start,
                    offset: 6.0,
                    match_width: false,
                    fit_height: false,
                };
                // A panel leaving for another fades and zooms out where it is; reka
                // also slides it toward the side the pointer went (`to-start`, `to-end`),
                // which is known only after it has left.
                let panel = root
                    .animation(opening(item.motion()))
                    .exit_animation(Animation::exit().fade(0.0).zoom(0.95))
                    .duration_ms(200.0);
                gui::deferred(placement, item.content(panel)).into_element()
            }
        })
    })
}

/// How a panel opens (`data-[motion=from-end]:slide-in-from-right-52`,
/// `from-start` from the left), on top of `fade-in-0 zoom-in-95
/// duration-200`: sliding only when the pointer came from another trigger.
fn opening(motion: Option<Motion>) -> Animation {
    let enter = Animation::enter().fade(0.0).zoom(0.95);
    match motion {
        Some(Motion::FromEnd) => enter.slide(Slide::X(Calc::px(SWITCH_SLIDE))),
        Some(Motion::FromStart) => enter.slide(Slide::X(Calc::px(-SWITCH_SLIDE))),
        _ => enter,
    }
}

/// What a link in a panel reads: the item whose panel it is in.
#[derive(Clone, Copy)]
struct Panel(NavigationMenuItem);

/// What a link is given: whether it is the current page, and what
/// following it does.
#[derive(Clone)]
pub struct NavigationMenuLinkProps {
    active: bool,
    on_select: gui::Callback<()>,
}

/// A link in a panel or in the list: `flex flex-col gap-1 rounded-sm p-2
/// text-sm hover:bg-accent focus:bg-accent`; `active` marks the current
/// page (`data-active:bg-accent/50`, `aria-current="page"`). Following it
/// runs `on_click` and closes the menu (a Command-click does not). Add its
/// title and description with `.child`.
#[must_use]
pub fn navigation_menu_link(active: bool, on_click: impl Into<gui::Callback<()>>) -> Part<NavigationMenuLinkProps> {
    let t = theme();
    navigation_menu_link_as(
        link_look(div(), &t, active).transition(Properties::COLOR | Properties::SHADOW),
        active,
        on_click,
    )
}

/// A link drawn as `look` (`NavigationMenuLink as-child`), such as
/// [`navigation_menu_trigger_style`] for a link in the list.
#[must_use]
pub fn navigation_menu_link_as(
    look: impl Into<Div>,
    active: bool,
    on_click: impl Into<gui::Callback<()>>,
) -> Part<NavigationMenuLinkProps> {
    let props = NavigationMenuLinkProps {
        active,
        on_select: on_click.into(),
    };
    Part::new(props, look.into().named("navigation-menu-link"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let panel = use_context::<Panel>();
            let nav = use_context::<NavigationMenu>();
            let focus = FocusHandle::new();
            // The `on_select` passed last, when it is picked.
            let on_select = cx.event(|p| &p.props.on_select);
            move || {
                let (active, root) = props.with(|p| (p.props.active, p.slot.0.clone()));
                let on_select = on_select.clone();
                let select = move |()| {
                    on_select.call(());
                    Dismiss::Close
                };
                match (panel, nav) {
                    (Some(Panel(item)), _) => item.link(root, focus, active, select),
                    (None, Some(nav)) => nav.link(root, focus, active, select),
                    (None, None) => root.on_click(move |_| {
                        select(());
                    }),
                }
                .into_element()
            }
        })
    })
}

fn link_look(d: Div, t: &Colors, active: bool) -> Div {
    let t = *t;
    d.display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(4.0)
        .radius(6.0)
        .padding(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .focusable()
        .when(active, |d| {
            d.background(t.accent.opacity(0.5)).text_color(t.accent_foreground)
        })
        .with_interaction(|d, i| lit(d, i, &t, Ring::new(4.0, t.ring.opacity(if t.dark { 0.2 } else { 0.1 }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::{Scenario, Signal, text};

    fn shows(s: &Scenario, what: &str) -> bool {
        s.last().nodes.iter().any(|n| match &n.content {
            gui::layout::Content::Text { content, .. } => &**content == what,
            _ => false,
        })
    }

    /// An item's content replaced in one update (keyed anew): the new one
    /// gives the viewport its panel before the old one leaves, and the old
    /// one's leaving takes back only its own, so the viewport draws the new
    /// panel.
    #[test]
    fn a_content_replaced_in_one_update_is_the_panel_the_viewport_draws() {
        let which = Signal::new(false);
        let mut s = Scenario::measuring(
            move || {
                let second = which.get();
                navigation_menu()
                    .default_value("one")
                    .child(navigation_menu_list().child(
                        navigation_menu_item("one").child(navigation_menu_trigger("One")).child(
                            div().key(gui::key_of(if second { "second" } else { "first" })).child(
                                navigation_menu_content().child(text(if second {
                                    "second panel"
                                } else {
                                    "first panel"
                                })),
                            ),
                        ),
                    ))
                    .into_element()
            },
            600,
            400,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        assert!(shows(&s, "first panel"));
        s.act("replace the content", || which.set(true));
        s.settle().expect("settled");
        assert!(shows(&s, "second panel"), "the viewport draws the new content's panel");
    }
}
