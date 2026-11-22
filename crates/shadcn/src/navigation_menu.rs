//! Navigation menu (`ui/navigation-menu`), as shadcn's demo uses it
//! (`:viewport="false"`): a row of triggers, each opening its own panel
//! below it. Pointing at a trigger opens its panel after 200ms (at once
//! when another is open), leaving trigger and panel closes it after 150ms;
//! a click toggles, Escape and a click outside close.
//!
//! The parts nest as in Vue: [`navigation_menu`] keeps which item is open,
//! its own or the caller's (`model`, `v-model`); each
//! [`navigation_menu_item`] holds its trigger and its content.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::clock::{TimerId, after, cancel};
use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Animation, Bind, Div, Element, Icon, Members, Model, NamedKey, Placement, Properties, Side, SideAlign, Signal,
    UseModel, component, div, icon, provide, text, use_context, use_model,
};

use crate::part::{Part, PartProps};
use crate::theme::{Theme, theme};

/// reka's `delayDuration`: how long the pointer rests on a trigger before its panel opens.
const OPEN_DELAY: Duration = Duration::from_millis(200);
/// How long after the pointer leaves a trigger and its panel the panel closes.
const CLOSE_DELAY: Duration = Duration::from_millis(150);

/// How far a panel slides in when the pointer moves to it from another
/// trigger (`slide-in-from-right-52`).
const SWITCH_SLIDE: f32 = 52.0 * gui::styled::UNIT;

/// What [`navigation_menu`] is given: the open item's value (reka's
/// `modelValue`; `None`: every panel closed).
#[derive(Clone, PartialEq, Default)]
pub struct NavigationMenuProps {
    value: Bind<Option<String>>,
}

impl Part<NavigationMenuProps> {
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
    pub fn on_change(self, f: impl Fn(Option<String>) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

/// What the parts below a [`navigation_menu`] read: which panel is open, the
/// one open before it, its items in tree order, and the timers that open
/// and close one.
#[derive(Clone)]
struct NavigationContext {
    open: UseModel<PartProps<NavigationMenuProps>, Option<String>>,
    /// The panel open just before this one, when one was (reka's `previousValue`).
    previous: Signal<Option<String>>,
    /// The items' values, in tree order as last laid out.
    items: Members<String>,
    open_timer: Rc<Cell<Option<TimerId>>>,
    close_timer: Rc<Cell<Option<TimerId>>>,
}

impl NavigationContext {
    fn is_open(&self, value: &str) -> bool {
        self.open.with(|v| v.as_deref() == Some(value))
    }

    fn any_open(&self) -> bool {
        self.open.get_untracked().is_some()
    }

    fn stop(timer: &Cell<Option<TimerId>>) {
        if let Some(t) = timer.take() {
            cancel(t);
        }
    }

    fn show(&self, value: &str) {
        Self::stop(&self.open_timer);
        Self::stop(&self.close_timer);
        if !gui::untracked(|| self.is_open(value)) {
            self.previous.set(self.open.get_untracked());
            self.open.set(Some(value.to_owned()));
        }
    }

    fn close(&self) {
        Self::stop(&self.open_timer);
        Self::stop(&self.close_timer);
        if self.any_open() {
            self.previous.set(None);
            self.open.set(None);
        }
    }

    /// Where `value`'s item is among the items.
    fn index(&self, value: &str) -> usize {
        self.items
            .with(|items| items.iter().position(|v| v == value).unwrap_or(0))
    }

    /// How `value`'s panel opens (`data-[motion=from-end]:slide-in-from-right-52`,
    /// `from-start` from the left), on top of `fade-in-0 zoom-in-95
    /// duration-200`: sliding only when the pointer came from another trigger.
    fn opening(&self, value: &str) -> Animation {
        let enter = Animation::enter().fade(0.0).zoom(0.95);
        let Some(from) = self.previous.get().filter(|p| p != value) else {
            return enter;
        };
        let (from, to) = (self.index(&from), self.index(value));
        enter.slide_x(Calc::px(if to > from { SWITCH_SLIDE } else { -SWITCH_SLIDE }))
    }

    fn pointer_enters(&self, value: &str) {
        Self::stop(&self.close_timer);
        if gui::untracked(|| self.is_open(value)) {
            return;
        }
        if self.any_open() {
            self.show(value);
            return;
        }
        Self::stop(&self.open_timer);
        let (me, value) = (self.clone(), value.to_owned());
        self.open_timer.set(Some(after(OPEN_DELAY, move || me.show(&value))));
    }

    fn pointer_leaves(&self) {
        Self::stop(&self.open_timer);
        Self::stop(&self.close_timer);
        if self.any_open() {
            let me = self.clone();
            self.close_timer.set(Some(after(CLOSE_DELAY, move || me.close())));
        }
    }
}

/// The root: `relative flex max-w-max flex-1 items-center justify-center`;
/// give it a [`navigation_menu_list`] of [`navigation_menu_item`]s.
#[must_use]
pub fn navigation_menu() -> Part<NavigationMenuProps> {
    let root = div()
        .named("navigation-menu")
        .relative()
        .flex()
        .flex_1()
        .items_center()
        .justify_center();
    Part::new(NavigationMenuProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let nav = NavigationContext {
                open: use_model(props, |p| &p.props.value),
                previous: Signal::labeled("navigation menu previous", None),
                items: Members::new(),
                open_timer: Rc::new(Cell::new(None)),
                close_timer: Rc::new(Cell::new(None)),
            };
            let (open_timer, close_timer) = (Rc::clone(&nav.open_timer), Rc::clone(&nav.close_timer));
            gui::on_cleanup(move || {
                NavigationContext::stop(&open_timer);
                NavigationContext::stop(&close_timer);
            });
            provide(nav);
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// `flex flex-1 items-center justify-center gap-1`.
#[must_use]
pub fn navigation_menu_list() -> Div {
    div()
        .named("navigation-menu-list")
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(1.0)
}

/// What an item and its trigger and content read: the item's value.
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationMenuItemProps {
    value: String,
}

#[derive(Clone)]
struct NavigationItem(String);

/// An item (`relative`), known by `value`: the anchor its content opens
/// against. Give it a [`navigation_menu_trigger`] and a
/// [`navigation_menu_content`], or a link.
#[must_use]
pub fn navigation_menu_item(value: &str) -> Part<NavigationMenuItemProps> {
    let props = NavigationMenuItemProps {
        value: value.to_owned(),
    };
    Part::new(props, div().named("navigation-menu-item").relative(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let nav = use_context::<NavigationContext>();
            let value = props.with_untracked(|p| p.props.value.clone());
            provide(NavigationItem(value.clone()));
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match &nav {
                    Some(n) => root.member(&n.items, value.clone()),
                    None => root,
                }
                .into_element()
            }
        })
    })
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
    d.inline_flex()
        .h(9.0)
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(if open { t.accent.opacity(0.5) } else { t.background })
        .when(open, |d| d.text_color(t.accent_foreground))
        .px(4.0)
        .py(2.0)
        .text_sm()
        .font_medium()
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .focusable()
        .hover(move |s| s.bg(t.accent).text_color(t.accent_foreground))
        .focus(move |s| s.bg(t.accent).text_color(t.accent_foreground))
        .focus_visible(move |s| s.ring(3.0, t.ring.opacity(0.5)))
        .disabled_style(|s| s.opacity(0.5))
}

/// The item's trigger, labelled `label`, with a chevron turning over while
/// its panel is open. Pointing at it opens the panel after 200ms (at once
/// when another is open); a click toggles; Escape closes.
#[must_use]
pub fn navigation_menu_trigger(label: &str) -> Part<()> {
    let root = div()
        .named("navigation-menu-trigger")
        // `w-max`: the trigger is as wide as its label on one line.
        .child(text(label.to_owned()).whitespace_nowrap());
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let nav = use_context::<NavigationContext>();
            let item = use_context::<NavigationItem>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let (Some(nav), Some(NavigationItem(value))) = (nav.clone(), item.clone()) else {
                    return root.into_element();
                };
                let open = nav.is_open(&value);
                let (hover, click, keys) = (nav.clone(), nav.clone(), nav);
                let (hover_value, click_value) = (value.clone(), value);
                trigger_look(root, open)
                    .aria_expanded(open)
                    .on_hover(move |inside| {
                        if inside {
                            hover.pointer_enters(&hover_value);
                        } else {
                            hover.pointer_leaves();
                        }
                    })
                    .on_click(move |_| {
                        if gui::untracked(|| click.is_open(&click_value)) {
                            click.close();
                        } else {
                            click.show(&click_value);
                        }
                    })
                    .on_key_down(move |ev| {
                        if ev.is(NamedKey::Escape) && keys.any_open() {
                            keys.close();
                            return true;
                        }
                        false
                    })
                    .child(
                        icon(Icon::ChevronDown)
                            .relative()
                            .top_len(gui::px(1.0))
                            .ml(1.0)
                            .size(3.0)
                            .transition()
                            .duration_ms(300.0)
                            .when(open, |i| i.rotate(180.0)),
                    )
                    .into_element()
            }
        })
    })
}

/// The item's panel (`p-2 pr-2.5`, and with no viewport `bg-popover
/// text-popover-foreground rounded-md border shadow overflow-hidden`),
/// below the item while it is open; add its links with `.child`.
#[must_use]
pub fn navigation_menu_content() -> Part<()> {
    let t = theme();
    let root = div()
        .named("navigation-menu-content")
        .p(2.0)
        .pr(2.5)
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .overflow_hidden()
        .rounded_md()
        .border()
        .border_color(t.border)
        .shadow_sm();
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let nav = use_context::<NavigationContext>();
            let item = use_context::<NavigationItem>();
            move || {
                let (Some(nav), Some(NavigationItem(value))) = (nav.clone(), item.clone()) else {
                    return Element::Empty;
                };
                if !nav.is_open(&value) {
                    return Element::Empty;
                }
                let root = props.with(|p| p.slot.0.clone());
                let placement = Placement::Anchored {
                    side: Side::Bottom,
                    align: SideAlign::Start,
                    offset: 6.0,
                    match_width: false,
                    fit_height: false,
                };
                let (hover, outside, keys) = (nav.clone(), nav.clone(), nav.clone());
                // A panel leaving for another fades and zooms out where it is; reka
                // also slides it toward the side the pointer went (`to-start`, `to-end`),
                // which is known only after it has left.
                let panel = root
                    .animate(nav.opening(&value))
                    .animate_out(Animation::exit().fade(0.0).zoom(0.95))
                    .duration_ms(200.0)
                    .on_hover(move |inside| {
                        if inside {
                            hover.pointer_enters(&value);
                        } else {
                            hover.pointer_leaves();
                        }
                    })
                    .on_click_outside(move || outside.close())
                    .on_key_down(move |ev| {
                        if !ev.is(NamedKey::Escape) {
                            return false;
                        }
                        keys.close();
                        true
                    });
                gui::deferred(placement, panel).into_element()
            }
        })
    })
}

/// A link in a panel: `flex flex-col gap-1 rounded-sm p-2 text-sm
/// hover:bg-accent focus:bg-accent`; `active` marks the current page
/// (`data-active:bg-accent/50`). Add its title and description with `.child`.
#[must_use]
pub fn navigation_menu_link(active: bool, on_click: impl Fn() + 'static) -> Div {
    let t = theme();
    link_look(div(), &t, active)
        .named("navigation-menu-link")
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .on_click(move |_| on_click())
}

fn link_look(d: Div, t: &Theme, active: bool) -> Div {
    let t = *t;
    d.flex_col()
        .gap(1.0)
        .rounded_sm()
        .p(2.0)
        .text_sm()
        .focusable()
        .when(active, |d| d.bg(t.accent.opacity(0.5)).text_color(t.accent_foreground))
        .hover(move |s| s.bg(t.accent).text_color(t.accent_foreground))
        .focus(move |s| s.bg(t.accent).text_color(t.accent_foreground))
        .focus_visible(move |s| s.ring(4.0, t.ring.opacity(if t.dark { 0.2 } else { 0.1 })))
}
