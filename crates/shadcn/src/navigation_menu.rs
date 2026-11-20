//! Navigation menu (`ui/navigation-menu`), as shadcn's demo uses it
//! (`:viewport="false"`): a row of triggers, each opening its own panel
//! below it. Pointing at a trigger opens its panel after 200ms (at once
//! when another is open), leaving trigger and panel closes it after 150ms;
//! a click toggles, Escape and a click outside close.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::clock::{TimerId, after, cancel};
use gui::prelude::*;
use gui::style::Calc;
use gui::{Animation, Div, Icon, NamedKey, Placement, Properties, Side, SideAlign, Signal, div, icon, text};

use crate::theme::{Theme, theme};

/// reka's `delayDuration`: how long the pointer rests on a trigger before its panel opens.
const OPEN_DELAY: Duration = Duration::from_millis(200);
/// How long after the pointer leaves a trigger and its panel the panel closes.
const CLOSE_DELAY: Duration = Duration::from_millis(150);

/// How far a panel slides in when the pointer moves to it from another
/// trigger (`slide-in-from-right-52`).
const SWITCH_SLIDE: f32 = 52.0 * gui::styled::UNIT;

/// Which panel is open, the one open before it, and the timers that open and
/// close one. Create it once, outside the render closure.
#[derive(Clone)]
pub struct NavigationMenuState {
    open: Signal<Option<String>>,
    /// The panel open just before this one, when one was (reka's `previousValue`).
    previous: Signal<Option<String>>,
    /// The triggers' values in the order they render.
    items: Rc<RefCell<Vec<String>>>,
    open_timer: Rc<Cell<Option<TimerId>>>,
    close_timer: Rc<Cell<Option<TimerId>>>,
}

impl Default for NavigationMenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationMenuState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: Signal::labeled("navigation menu open", None),
            previous: Signal::labeled("navigation menu previous", None),
            items: Rc::default(),
            open_timer: Rc::new(Cell::new(None)),
            close_timer: Rc::new(Cell::new(None)),
        }
    }

    /// The open panel's value (reactive).
    #[must_use]
    pub fn open_value(&self) -> Option<String> {
        self.open.get()
    }

    fn is_open(&self, value: &str) -> bool {
        self.open.get().as_deref() == Some(value)
    }

    fn stop(timer: &Cell<Option<TimerId>>) {
        if let Some(t) = timer.take() {
            cancel(t);
        }
    }

    pub fn show(&self, value: &str) {
        Self::stop(&self.open_timer);
        Self::stop(&self.close_timer);
        if !self.is_open(value) {
            self.previous.set(self.open.get());
            self.open.set(Some(value.to_owned()));
        }
    }

    pub fn close(&self) {
        Self::stop(&self.open_timer);
        Self::stop(&self.close_timer);
        if self.open.get().is_some() {
            self.previous.set(None);
            self.open.set(None);
        }
    }

    /// Where `value`'s trigger renders among the triggers, registering it the
    /// first time.
    fn index(&self, value: &str) -> usize {
        let mut items = self.items.borrow_mut();
        items.iter().position(|v| v == value).unwrap_or_else(|| {
            items.push(value.to_owned());
            items.len().saturating_sub(1)
        })
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
        if self.is_open(value) {
            return;
        }
        if self.open.get().is_some() {
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
        if self.open.get().is_some() {
            let me = self.clone();
            self.close_timer.set(Some(after(CLOSE_DELAY, move || me.close())));
        }
    }
}

/// `relative flex max-w-max flex-1 items-center justify-center`.
#[must_use]
pub fn navigation_menu() -> Div {
    div()
        .named("navigation-menu")
        .relative()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
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

/// `relative`.
#[must_use]
pub fn navigation_menu_item() -> Div {
    div().named("navigation-menu-item").relative()
}

/// `navigationMenuTriggerStyle`: `inline-flex h-9 w-max items-center
/// justify-center rounded-md bg-background px-4 py-2 text-sm font-medium
/// hover:bg-accent focus:bg-accent data-[state=open]:bg-accent/50`, for a
/// trigger or a link that looks like one.
#[must_use]
pub fn navigation_menu_trigger_style(open: bool) -> Div {
    let t = theme();
    div()
        .inline_flex()
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

/// A trigger labelled `label` with a chevron turning over while open;
/// `content` (a [`navigation_menu_content`]) opens below it.
#[must_use]
pub fn navigation_menu_trigger(nav: &NavigationMenuState, value: &str, label: &str, content: Div) -> Div {
    let open = nav.is_open(value);
    nav.index(value);
    let (hover, click, keys) = (nav.clone(), nav.clone(), nav.clone());
    let (hover_value, click_value) = (value.to_owned(), value.to_owned());
    let placement = Placement::Anchored {
        side: Side::Bottom,
        align: SideAlign::Start,
        offset: 6.0,
        match_width: false,
        fit_height: false,
    };
    // A panel leaving for another fades and zooms out where it is; reka
    // also slides it toward the side the pointer went (`to-start`, `to-end`),
    // which is known only after it has left.
    let panel = content
        .animate(nav.opening(value))
        .animate_out(Animation::exit().fade(0.0).zoom(0.95))
        .duration_ms(200.0)
        .on_hover({
            let nav = nav.clone();
            let value = value.to_owned();
            move |inside| {
                if inside {
                    nav.pointer_enters(&value);
                } else {
                    nav.pointer_leaves();
                }
            }
        })
        .on_click_outside({
            let nav = nav.clone();
            move || nav.close()
        })
        .on_key_down({
            let nav = nav.clone();
            move |ev| {
                if !ev.is(NamedKey::Escape) {
                    return false;
                }
                nav.close();
                true
            }
        });
    navigation_menu_trigger_style(open)
        .named("navigation-menu-trigger")
        .on_hover(move |inside| {
            if inside {
                hover.pointer_enters(&hover_value);
            } else {
                hover.pointer_leaves();
            }
        })
        .on_click(move |_| {
            if click.is_open(&click_value) {
                click.close();
            } else {
                click.show(&click_value);
            }
        })
        .on_key_down(move |ev| {
            if ev.is(NamedKey::Escape) && keys.open.get().is_some() {
                keys.close();
                return true;
            }
            false
        })
        // `w-max`: the trigger is as wide as its label on one line.
        .child(text(label.to_owned()).whitespace_nowrap())
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
        .child(open.then(|| gui::deferred(placement, panel)))
}

/// The panel (`p-2 pr-2.5`, and with no viewport `bg-popover
/// text-popover-foreground rounded-md border shadow overflow-hidden`).
#[must_use]
pub fn navigation_menu_content() -> Div {
    let t = theme();
    div()
        .named("navigation-menu-content")
        .p(2.0)
        .pr(2.5)
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .overflow_hidden()
        .rounded_md()
        .border()
        .border_color(t.border)
        .shadow_sm()
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
