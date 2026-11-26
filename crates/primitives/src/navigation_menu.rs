//! A site's navigation: a row of triggers, each showing a panel of links
//! (reka's `NavigationMenu`; research §8, "NavigationMenu").
//!
//! - **Value.** The root holds the open item's value, none for every panel
//!   closed (reka's `modelValue`, `''`), kept by the caller (its write takes
//!   effect at once) or by the root from a default. It remembers the value
//!   open before ([`NavigationMenu::motion`]).
//! - **Hover** (the mouse; a touch or pen tap is a click): the pointer moving
//!   on a trigger while every panel is closed opens it after
//!   [`OPEN_DELAY`] (200 ms), or [`SKIPPED_DELAY`] (150 ms) within
//!   [`SKIP_DELAY`] (300 ms) of a panel closed by the pointer leaving; while
//!   one is open, entering another trigger switches at once and drops the
//!   close on its way. Leaving a trigger closes after [`CLOSE_DELAY`]
//!   (150 ms); entering the panel drops that; leaving the panel closes after
//!   150 ms too, unless the caller turned that off. Every request goes
//!   through one timer: a later one replaces the earlier (reka's debounce).
//! - **Click** (and Enter) toggles at once, but a mouse click within
//!   [`CLICK_AFTER_MOVE`] (300 ms) of a move that asked to open is ignored,
//!   so a panel the pointer just opened is not closed by the click that
//!   follows. After Escape, or after a click closed it, the pointer moving
//!   on the trigger does not open it again until it leaves and comes back.
//!   reka sets its "closed by a click" flag after the value changed, so it
//!   is set when a click *opens* the panel and a click that closes it lets
//!   the hover open it again 200 ms later; here it is set when the click
//!   closes it, as its name and the Escape rule say (research §4 item 13).
//!   Space toggles too (the engine presses the focused trigger).
//! - **Keys** on a trigger or a link of the list: every arrow moves to the
//!   next or previous one of them, in either axis, no wrap; Home and End to
//!   the ends. reka ignores the reading direction there; here ArrowLeft and
//!   ArrowRight swap right to left, as everywhere (research §4 item 13).
//!   ArrowDown on the open trigger of a horizontal menu (ArrowRight, or
//!   ArrowLeft right to left, in a vertical one) focuses the panel's first
//!   link. In a panel, the arrows, Home and End move among its links, no
//!   wrap.
//! - **Focus.** Opening moves no focus: the panel has no focus scope. Tab
//!   from an open trigger enters its panel, Tab past its last link reaches
//!   the next item, Shift+Tab from the next item its last link: a panel
//!   declared in its item follows it in `gui`'s Tab order (a floating
//!   subtree comes right after its owner's), and a viewport's follows the
//!   open trigger ([`gui::Deferred::tab_after`]), which is what reka's
//!   focus proxy does. A panel that is not open (kept while it leaves) has
//!   no Tab stops (reka's `removeFromTabOrder`).
//! - **Closing**: Escape (wherever focus is, while the panel is the top
//!   layer: focus goes back to its trigger); a press outside, but not on a
//!   trigger or a link of the menu; focus leaving the menu (focus on its
//!   triggers and links stays); picking a link at any depth closes the
//!   whole menu, nested menus included, focus going back to the root
//!   item's trigger (its handler may veto, and a Command-click does not
//!   close: it opens elsewhere).
//! - **AT.** The root is a `navigation` landmark, its list a list of items.
//!   A trigger has `aria-expanded`, and controls and owns its panel while it
//!   is shown (the panel is a layer, so it is read in the landmark after its
//!   trigger, as reka's in its `li`); no `aria-haspopup` (it is no menu).
//!   The panel has no role and is named by its trigger. An active link is
//!   `aria-current="page"`.
//!
//! - **Viewport** ([`NavigationMenu::use_viewport`],
//!   [`NavigationMenu::viewport`], reka's `NavigationMenuViewport`): one
//!   surface below the menu draws the open item's panel
//!   ([`NavigationMenu::open_item`]); the pointer in it holds the panel
//!   open, leaving it closes it after 150 ms; its size is the panel's
//!   ([`NavigationMenu::viewport_size`]).
//! - **Nested menus** ([`NavigationMenu::sub`], reka's `NavigationMenuSub`):
//!   a menu in a panel, its own value; its triggers switch at once on
//!   hover, the pointer leaving closes nothing, a press outside its panel
//!   does not close it, and Escape closes its panel alone.
//! - **Space** toggles once per press: a held Space's repeats press
//!   nothing (`gui::window::Input::KeyRepeat`).
//!
//! The indicator's place is [`NavigationMenu::active_trigger`].

use gui::engine::{WorldCell, WorldRefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::focus::FocusId;
use gui::{
    AriaCurrent, Axis, Bounds, Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, KeyEvent, Layer, Members,
    NamedKey, Role, Signal, UseModel,
};

use crate::collection::{Collection, Item};
use crate::direction::{ReadDirection, use_direction};
use crate::keymap::{Arrows, Intent, Keymap, Wrap};

/// How long the pointer moves on a trigger before its panel opens (reka's
/// `delayDuration`).
pub const OPEN_DELAY: Duration = Duration::from_millis(200);
/// How long after a panel the pointer closed the next opens sooner (reka's
/// `skipDelayDuration`).
pub const SKIP_DELAY: Duration = Duration::from_millis(300);
/// The delay within [`SKIP_DELAY`], and the delay of every request while a
/// panel is open (reka's 150 ms).
pub const SKIPPED_DELAY: Duration = Duration::from_millis(150);
/// How long after the pointer leaves a trigger or a panel it closes.
pub const CLOSE_DELAY: Duration = Duration::from_millis(150);
/// How long a click is ignored after a move asked to open (reka's
/// `hasPointerMoveOpened`).
pub const CLICK_AFTER_MOVE: Duration = Duration::from_millis(300);

/// The root's value: the open item's, none for every panel closed.
pub trait NavValue {
    /// Whether `value`'s panel is open (tracked).
    fn is(&self, value: &str) -> bool;
    /// The open item's value, untracked.
    fn current(&self) -> Option<String>;
    fn set(&self, value: Option<String>);
}

impl NavValue for Signal<Option<String>> {
    fn is(&self, value: &str) -> bool {
        self.with(|v| v.as_deref() == Some(value))
    }

    fn current(&self) -> Option<String> {
        self.get_untracked()
    }

    fn set(&self, value: Option<String>) {
        if self.with_untracked(|v| *v != value) {
            Signal::set(self, value);
        }
    }
}

impl<P: 'static> NavValue for UseModel<P, Option<String>> {
    fn is(&self, value: &str) -> bool {
        self.with(|v| v.as_deref() == Some(value))
    }

    fn current(&self) -> Option<String> {
        self.get_untracked()
    }

    fn set(&self, value: Option<String>) {
        UseModel::set(self, value);
    }
}

/// What a navigation menu is set up with (reka's root props).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavOptions {
    /// [`OPEN_DELAY`] by default.
    pub delay: Duration,
    /// [`SKIP_DELAY`] by default.
    pub skip_delay: Duration,
    /// The list's axis (horizontal): which arrow enters an open panel.
    pub orientation: Axis,
    /// A mouse click does not toggle (a touch or pen tap, Enter and Space
    /// still do: reka checks `pointerType === 'mouse'`).
    pub disable_click: bool,
    /// The pointer neither opens nor closes a panel from a trigger.
    pub disable_hover: bool,
    /// The pointer leaving a panel does not close it.
    pub disable_pointer_leave_close: bool,
}

impl NavOptions {
    pub const DEFAULT: Self = Self {
        delay: OPEN_DELAY,
        skip_delay: SKIP_DELAY,
        orientation: Axis::Horizontal,
        disable_click: false,
        disable_hover: false,
        disable_pointer_leave_close: false,
    };
}

impl Default for NavOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Which way an item's panel moves as the open item changes (reka's
/// `data-motion`): toward the item now open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    /// Coming in from the end side: the item before was earlier in the list.
    FromEnd,
    /// Coming in from the start side.
    FromStart,
    /// Leaving toward the start: the item now open is later.
    ToStart,
    /// Leaving toward the end.
    ToEnd,
}

/// Its cells are the world's that made the menu: a check writes none of a
/// live one's (ADR-0034, S7). The boxes are written by the menu's own
/// layout callback on its triggers and links, bounded by those laid out.
#[derive(Clone)]
struct Nav {
    value: Rc<dyn NavValue>,
    options: NavOptions,
    direction: ReadDirection,
    previous: Signal<Option<String>>,
    /// The items, in tree order, for the motion.
    order: Members<Rc<str>>,
    /// The list's triggers and links, for the arrows.
    top: Collection<()>,
    /// Where the triggers and links of the menu were laid out: a press on
    /// one is not outside a panel.
    boxes: WorldRefCell<Vec<(FocusId, Bounds)>>,
    /// The open item's trigger box (tracked: the indicator's).
    active: Signal<Option<Bounds>>,
    timer: WorldCell<Option<TimerId>>,
    /// A panel the pointer closed opens the next sooner, for a while.
    skipped: WorldCell<bool>,
    skip_timer: WorldCell<Option<TimerId>>,
    /// A pending close the pointer's switch to another trigger drops.
    skip_next_close: WorldCell<bool>,
    /// The menu this one is nested in (reka's `NavigationMenuSub`): its
    /// triggers switch at once on hover, the pointer leaving closes
    /// nothing, and a press outside its panel does not close it.
    parent: Option<NavigationMenu>,
    /// Its items by value, registered in their setups.
    items: gui::Registry<(Rc<str>, NavigationMenuItem)>,
    /// The menus nested in its panels.
    subs: gui::Registry<NavigationMenu>,
    /// A viewport shows the open panel (reka's `NavigationMenuViewport`).
    /// Viewport parts mounted (a count: one replacing another registers
    /// before the one it replaces leaves).
    viewport: Signal<u32>,
    /// The open panel's size, as laid out in the viewport.
    viewport_size: Signal<Option<(f32, f32)>>,
}

/// A navigation menu: made once in its root's setup, `Copy`.
#[derive(Clone, Copy)]
pub struct NavigationMenu {
    nav: Signal<Nav>,
}

impl PartialEq for NavigationMenu {
    fn eq(&self, other: &Self) -> bool {
        self.nav == other.nav
    }
}

impl std::fmt::Debug for NavigationMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationMenu").finish_non_exhaustive()
    }
}

fn cancel(timer: &WorldCell<Option<TimerId>>) {
    if let Some(t) = timer.take() {
        clock::cancel(t);
    }
}

impl NavigationMenu {
    /// A navigation menu whose open item `value` holds, set up as
    /// `options`; called in the root's setup. Its timers end with it.
    #[must_use]
    #[track_caller]
    pub fn new(value: impl NavValue + 'static, options: NavOptions) -> Self {
        Self::make(value, options, None)
    }

    /// A menu nested in a panel of this one (reka's `NavigationMenuSub`),
    /// its open item `value` holds, its list along `orientation`; called in
    /// the sub part's setup. Its triggers switch at once on hover, the
    /// pointer leaving closes nothing, and a press outside its panel does
    /// not close it; Escape closes its panel alone.
    #[must_use]
    #[track_caller]
    pub fn sub(self, value: impl NavValue + 'static, orientation: Axis) -> Self {
        let options = NavOptions {
            orientation,
            ..self.with(|n| n.options)
        };
        let sub = Self::make(value, options, Some(self));
        self.with(|n| n.subs).register(sub);
        sub
    }

    #[track_caller]
    fn make(value: impl NavValue + 'static, options: NavOptions, parent: Option<NavigationMenu>) -> Self {
        let me = Self {
            nav: Signal::new(Nav {
                value: Rc::new(value),
                options,
                direction: use_direction(None),
                previous: Signal::labeled("navigation menu previous", None),
                order: Members::new(),
                top: Collection::new(),
                boxes: WorldRefCell::new("a navigation menu's trigger boxes", Vec::new()),
                active: Signal::labeled("navigation menu active trigger", None),
                timer: WorldCell::new("a navigation menu's timer", None),
                skipped: WorldCell::new("whether a navigation menu skips its delay", false),
                skip_timer: WorldCell::new("a navigation menu's skip timer", None),
                skip_next_close: WorldCell::new("whether a navigation menu drops its next close", false),
                parent,
                items: gui::Registry::new(),
                subs: gui::Registry::new(),
                viewport: Signal::labeled("navigation menu viewport", 0),
                viewport_size: Signal::labeled("navigation menu viewport size", None),
            }),
        };
        gui::on_cleanup(move || {
            if me.nav.is_alive() {
                me.with(|n| {
                    cancel(&n.timer);
                    cancel(&n.skip_timer);
                });
            }
        });
        me
    }

    fn with<R>(self, f: impl FnOnce(&Nav) -> R) -> R {
        self.nav.with_untracked(f)
    }

    fn value(self) -> Rc<dyn NavValue> {
        self.with(|n| Rc::clone(&n.value))
    }

    /// Whether `value`'s panel is open (tracked).
    #[must_use]
    pub fn is_open(self, value: &str) -> bool {
        self.nav.is_alive() && self.value().is(value)
    }

    fn any_open(self) -> bool {
        self.value().current().is_some()
    }

    /// Open `value`'s panel now, or close every one (`None`), remembering
    /// what was open.
    pub fn select(self, value: Option<String>) {
        self.with(|n| cancel(&n.timer));
        let v = self.value();
        let now = v.current();
        if now != value {
            self.with(|n| n.previous.set(now));
            v.set(value);
        }
    }

    /// Close every panel.
    pub fn close(self) {
        self.select(None);
    }

    /// Close every panel of this menu and of the menus nested in it.
    fn close_tree(self) {
        if !self.nav.is_alive() {
            return;
        }
        self.close();
        let subs: Vec<NavigationMenu> = self.with(|n| n.subs).to_vec_untracked();
        for sub in subs {
            sub.close_tree();
        }
    }

    /// The request every hover makes: `Some(Some(v))` opens `v`, `Some(None)`
    /// closes, `None` only drops what is pending (reka's debounced setter).
    fn request(self, value: Option<Option<String>>) {
        self.with(|n| cancel(&n.timer));
        let Some(value) = value else { return };
        let (skipped, options) = self.with(|n| (n.skipped.get(), n.options));
        let delay = if self.any_open() || skipped {
            SKIPPED_DELAY
        } else {
            options.delay
        };
        // Owned by nothing: a scope that happens to run the event (an
        // item re-rendered by its highlight) must not cancel it.
        let timer = gui::detached(|| {
            clock::after(delay, move || {
                if !self.nav.is_alive() {
                    return;
                }
                self.with(|n| n.timer.set(None));
                if value.is_none() && self.with(|n| n.skip_next_close.replace(false)) {
                    return;
                }
                let closing = value.is_none();
                self.select(value.clone());
                if closing {
                    self.skip_delay();
                }
            })
        });
        self.with(|n| n.timer.set(Some(timer)));
    }

    /// For a while after the pointer closed a panel, the next opens sooner.
    fn skip_delay(self) {
        let window = self.with(|n| {
            n.skipped.set(true);
            cancel(&n.skip_timer);
            n.options.skip_delay
        });
        // Owned by nothing: a scope that happens to run the event (an
        // item re-rendered by its highlight) must not cancel it.
        let timer = gui::detached(|| {
            clock::after(window, move || {
                if self.nav.is_alive() {
                    self.with(|n| {
                        n.skipped.set(false);
                        n.skip_timer.set(None);
                    });
                }
            })
        });
        self.with(|n| n.skip_timer.set(Some(timer)));
    }

    fn is_sub(self) -> bool {
        self.with(|n| n.parent.is_some())
    }

    fn trigger_enter(self, value: &str) {
        if self.is_sub() {
            self.select(Some(value.to_owned()));
            return;
        }
        if self.any_open() {
            self.with(|n| n.skip_next_close.set(true));
            self.select(Some(value.to_owned()));
        } else {
            self.request(Some(Some(value.to_owned())));
        }
    }

    fn trigger_leave(self) {
        if self.is_sub() {
            return;
        }
        self.with(|n| n.skip_next_close.set(false));
        self.request(Some(None));
    }

    fn content_enter(self) {
        if !self.is_sub() {
            self.request(None);
        }
    }

    fn content_leave(self) {
        if !self.is_sub() && !self.with(|n| n.options.disable_pointer_leave_close) {
            self.with(|n| n.skip_next_close.set(false));
            self.request(Some(None));
        }
    }

    /// How `value`'s panel moves now (tracked; none: the menu opened from
    /// closed or closed altogether, which reka animates in no direction).
    /// Right to left reverses the list's order, so the motion keeps to the
    /// screen's left and right.
    #[must_use]
    pub fn motion(self, value: &str) -> Option<Motion> {
        let (previous, order, direction) = self.with(|n| (n.previous, n.order.clone(), n.direction));
        let open = self.value();
        let selected = open.is(value);
        let before = previous.with(|p| p.as_deref() == Some(value));
        let now = open.current();
        let prev = previous.get();
        let index = |v: Option<&str>| v.and_then(|v| order.with(|o| o.iter().position(|i| &**i == v)));
        let (mut at, mut was) = (index(now.as_deref()), index(prev.as_deref()));
        if direction.get() == gui::Direction::Rtl {
            let len = order.len();
            let flip = |i: Option<usize>| i.and_then(|i| len.checked_sub(1).and_then(|l| l.checked_sub(i)));
            at = flip(at);
            was = flip(was);
        }
        let (at, was) = (at?, was?);
        if at == was {
            return None;
        }
        if selected {
            return Some(if at > was { Motion::FromEnd } else { Motion::FromStart });
        }
        if before {
            return Some(if at > was { Motion::ToStart } else { Motion::ToEnd });
        }
        None
    }

    /// The open item's trigger box, for an indicator (tracked); kept as it
    /// was while every panel is closed.
    #[must_use]
    pub fn active_trigger(self) -> Option<Bounds> {
        self.with(|n| n.active).get()
    }

    /// The menu at the top of this one's nesting.
    #[must_use]
    pub fn root_menu(self) -> Self {
        let mut at = self;
        while let Some(p) = at.with(|n| n.parent) {
            at = p;
        }
        at
    }

    /// Whether `focus` is on a trigger or a link of this menu, of one of
    /// its panels, or of a menu nested in them.
    #[must_use]
    pub fn holds(self, focus: FocusId) -> bool {
        if !self.nav.is_alive() {
            return false;
        }
        if self.with(|n| n.top.index_of(focus).is_some()) {
            return true;
        }
        let items: Vec<NavigationMenuItem> = self.with(|n| n.items).with_untracked(|l| l.map(|(_, i)| *i).collect());
        if items
            .iter()
            .any(|i| i.trigger.id() == focus || i.links.get_untracked().index_of(focus).is_some())
        {
            return true;
        }
        let subs: Vec<NavigationMenu> = self.with(|n| n.subs).to_vec_untracked();
        subs.into_iter().any(|s| s.holds(focus))
    }

    /// The item of `value`, while it is mounted.
    #[must_use]
    pub fn item(self, value: &str) -> Option<NavigationMenuItem> {
        self.with(|n| n.items)
            .with(|mut l| l.find(|(v, _)| &**v == value).map(|(_, i)| *i))
    }

    /// The open item (tracked): the one whose panel a viewport shows.
    #[must_use]
    pub fn open_item(self) -> Option<NavigationMenuItem> {
        if !self.nav.is_alive() {
            return None;
        }
        // Tracked through the value and the items mounted: an item opening,
        // or the open one mounting, renders the viewport.
        let open = self.with(|n| n.items).to_vec_tracked();
        open.into_iter().find(|(v, _)| self.value().is(v)).map(|(_, i)| i)
    }

    /// Show the panels in a viewport from now on (reka's
    /// `NavigationMenuViewport`); called in the viewport part's setup, it
    /// lasts while the part does. The items' content parts then draw
    /// nothing where they are declared: the viewport draws the open one.
    #[track_caller]
    pub fn use_viewport(self) {
        let viewport = self.with(|n| n.viewport);
        // Counted from its mount to its unmount (a setup, as a render,
        // writes only what it made): an unmount undoes only a mount that
        // counted.
        gui::while_mounted(move || {
            viewport.update(|n| *n = n.saturating_add(1));
            move || {
                if viewport.is_alive() {
                    viewport.update(|n| *n = n.saturating_sub(1));
                }
            }
        });
    }

    /// Whether a viewport shows the panels (tracked).
    #[must_use]
    pub fn has_viewport(self) -> bool {
        self.nav.is_alive() && self.with(|n| n.viewport).get() > 0
    }

    /// The open panel's size, as laid out in the viewport (tracked; kept
    /// while every panel is closed): a look sizing or animating the
    /// viewport reads it (reka's `--reka-navigation-menu-viewport-width`,
    /// `-height`).
    #[must_use]
    pub fn viewport_size(self) -> Option<(f32, f32)> {
        self.with(|n| n.viewport_size).get()
    }

    /// `d` as the viewport, the surface the open panel is drawn in: the
    /// pointer in it holds the panel open, leaving it closes it after
    /// [`CLOSE_DELAY`], as the panel's own box does. Place it in a
    /// `gui::deferred` declared in the root whose Tab stops follow the open
    /// trigger ([`gui::Deferred::tab_after`] with
    /// [`NavigationMenuItem::trigger_key`]), so Tab from the trigger enters
    /// the panel and Tab past its end reaches the next item (what reka's
    /// focus proxy does).
    #[must_use]
    pub fn viewport(self, d: Div) -> Div {
        d.on_hover(move |inside| {
            if inside {
                self.content_enter();
            } else {
                self.content_leave();
            }
        })
    }

    /// `d` as the root: the `navigation` landmark.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(Role::Navigation)
    }

    /// `d` as the list of items.
    #[must_use]
    pub fn list(self, d: Div) -> Div {
        d.role(Role::List)
    }

    fn measured(self, focus: FocusId, b: Bounds) {
        if !self.nav.is_alive() {
            return;
        }
        self.with(|n| {
            n.boxes.with_mut(|boxes| {
                match boxes.iter_mut().find(|(f, _)| *f == focus) {
                    Some(slot) => slot.1 = b,
                    None => boxes.push((focus, b)),
                }
                // Kept for the triggers and links laid out: those that left
                // are let go once they outnumber them.
                let laid = n.top.len();
                if boxes.len() > laid.saturating_mul(2).saturating_add(8) {
                    boxes.retain(|(f, _)| n.top.index_of(*f).is_some());
                }
            });
        });
    }

    /// Whether `(x, y)` is on a trigger or a link of the menu, as laid out.
    fn on_menu(self, x: f32, y: f32) -> bool {
        let top = self.with(|n| n.top.clone());
        self.with(|n| {
            n.boxes.with(|boxes| {
                boxes.iter().any(|(f, b)| {
                    top.index_of(*f).is_some() && x >= b.x && x <= b.right() && y >= b.y && y <= b.bottom()
                })
            })
        })
    }

    /// The keys of a trigger or link of the list, from `from`.
    fn top_key(self, from: FocusHandle, ev: &KeyEvent) -> bool {
        if gui::untracked(gui::focus::focused) != Some(from.id()) {
            return false;
        }
        let (top, direction) = self.with(|n| (n.top.clone(), n.direction));
        let keymap = Keymap::new(Arrows::Both, Wrap::Stop);
        let Some(intent) = keymap.intent(ev, direction.get()) else {
            return false;
        };
        let to = top.with(|list| {
            let at = list.iter().position(|i| i.focus == from);
            keymap.step(list, at, intent).and_then(|i| list.get(i)).map(|i| i.focus)
        });
        if let Some(f) = to {
            gui::focus::set_focus(Some(f.id()), true);
        }
        true
    }

    /// `d` as a link of the list (not in a panel), focused through `focus`,
    /// current when `active`; picking it runs `on_select`, whose answer
    /// closes the menu or keeps it.
    #[must_use]
    pub fn link(
        self,
        d: Div,
        focus: FocusHandle,
        active: bool,
        on_select: impl Into<gui::Callback<(), Dismiss>>,
    ) -> Div {
        let top = self.with(|n| n.top.clone());
        let d = top
            .item(d, Item::new((), focus))
            .on_key_down(move |ev| self.top_key(focus, ev))
            .on_measured(move |b| self.measured(focus.id(), *b));
        self.link_part(d, active, on_select.into(), gui::Callback::new(|()| {}))
    }

    /// A link picked at any depth closes the whole navigation menu, nested
    /// menus included (reka sends its dismiss up to the root content).
    /// `closing` runs before it closes.
    fn link_part(self, d: Div, active: bool, on_select: gui::Callback<(), Dismiss>, closing: gui::Callback<()>) -> Div {
        let d = d.role(Role::Link);
        let d = if active { d.aria_current(AriaCurrent::Page) } else { d };
        d.on_click(move |ev| {
            if on_select.call(()).unwrap_or_default() == Dismiss::Close && !ev.modifiers.command && self.nav.is_alive()
            {
                closing.call(());
                self.root_menu().close_tree();
            }
        })
    }
}

/// The world's that made the item: a check writes none of a live one's.
#[derive(Clone, Default)]
struct Flags {
    /// Escape closed it: the pointer does not open it again until it
    /// leaves the trigger and comes back.
    escape_closed: WorldCell<bool>,
    /// A click closed it: the same.
    click_closed: WorldCell<bool>,
    /// A move asked to open it just now: a click is ignored.
    moved_open: WorldCell<bool>,
    moved_timer: WorldCell<Option<TimerId>>,
    /// Where the pointer last moved on the trigger: an event at the same
    /// place, which `gui` sends again after a frame, is no move (a browser
    /// sends no `pointermove` then).
    at: WorldCell<Option<(f32, f32)>>,
    /// A touch or pen is down on the trigger: its click is a tap, not the
    /// mouse's.
    touched: WorldCell<bool>,
}

/// One item of a navigation menu, a trigger and its panel (reka's
/// `NavigationMenuItem`): made once in its part's setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationMenuItem {
    nav: NavigationMenu,
    value: Signal<Rc<str>>,
    trigger: FocusHandle,
    link: crate::relation::Linked,
    links: Signal<Collection<()>>,
    flags: Signal<Flags>,
}

impl std::fmt::Debug for Flags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flags").finish_non_exhaustive()
    }
}

impl NavigationMenuItem {
    /// The item of `value` in `nav`; called in the item part's setup.
    #[must_use]
    #[track_caller]
    pub fn new(nav: NavigationMenu, value: impl Into<Rc<str>>) -> Self {
        let me = Self {
            nav,
            value: Signal::new(value.into()),
            trigger: FocusHandle::new(),
            link: crate::relation::Linked::new(crate::relation::Points::Controls),
            links: Signal::new(Collection::new()),
            flags: Signal::new(Flags::default()),
        };
        let value = me.value();
        nav.with(|n| n.items).register((Rc::clone(&value), me));
        gui::on_cleanup(move || {
            if me.flags.is_alive() {
                me.flags.with_untracked(|f| {
                    if let Some(t) = f.moved_timer.take() {
                        clock::cancel(t);
                    }
                });
            }
        });
        me
    }

    /// The key its trigger's node carries: a viewport's Tab stops follow it
    /// ([`NavigationMenu::viewport`]).
    #[must_use]
    pub fn trigger_key(self) -> gui::ElementKey {
        gui::key_of(&format!("navigation-menu-trigger {}", self.value()))
    }

    /// The menu it is an item of.
    #[must_use]
    pub fn menu(self) -> NavigationMenu {
        self.nav
    }

    fn value(self) -> Rc<str> {
        self.value.get_untracked()
    }

    fn flags<R>(self, f: impl FnOnce(&Flags) -> R) -> R {
        self.flags.with_untracked(f)
    }

    /// Whether its panel is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.value.is_alive() && self.nav.is_open(&self.value())
    }

    /// How its panel moves now (tracked).
    #[must_use]
    pub fn motion(self) -> Option<Motion> {
        self.nav.motion(&self.value())
    }

    /// The trigger's focus.
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// `d` as the item: it takes its place in the menu's order.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let order = self.nav.with(|n| n.order.clone());
        d.role(Role::ListItem).member(&order, self.value())
    }

    fn open_toggle(self) {
        let open = gui::untracked(|| self.is_open());
        self.nav.select((!open).then(|| self.value().to_string()));
        self.flags(|f| f.click_closed.set(open));
    }

    fn moved(self, x: f32, y: f32) {
        if self.flags(|f| f.at.replace(Some((x, y)))) == Some((x, y)) {
            return;
        }
        if self.nav.with(|n| n.options.disable_hover) {
            return;
        }
        let blocked = self.flags(|f| f.click_closed.get() || f.escape_closed.get() || f.moved_open.get());
        if blocked {
            return;
        }
        self.nav.trigger_enter(&self.value());
        let flags = self.flags;
        // Owned by nothing: a scope that happens to run the event (an
        // item re-rendered by its highlight) must not cancel it.
        let timer = gui::detached(|| {
            clock::after(CLICK_AFTER_MOVE, move || {
                if flags.is_alive() {
                    flags.with_untracked(|f| {
                        f.moved_open.set(false);
                        f.moved_timer.set(None);
                    });
                }
            })
        });
        self.flags(|f| {
            f.moved_open.set(true);
            if let Some(t) = f.moved_timer.replace(Some(timer)) {
                clock::cancel(t);
            }
        });
    }

    fn hovered(self, inside: bool) {
        if self.nav.with(|n| n.options.disable_hover) {
            return;
        }
        if inside {
            self.flags(|f| {
                f.click_closed.set(false);
                f.escape_closed.set(false);
            });
        } else {
            self.nav.trigger_leave();
            self.flags(|f| {
                f.at.set(None);
                f.moved_open.set(false);
                if let Some(t) = f.moved_timer.take() {
                    clock::cancel(t);
                }
            });
        }
    }

    /// Focus the first (or last) link of the open panel.
    fn enter_panel(self, intent: Intent) {
        let links = self.links.get_untracked();
        let to = links.with(|list| {
            Keymap::LINE
                .step(list, None, intent)
                .and_then(|i| list.get(i))
                .map(|i| i.focus)
        });
        if let Some(f) = to {
            gui::focus::set_focus(Some(f.id()), true);
        }
    }

    fn on_trigger_key(self, ev: &KeyEvent) -> bool {
        let (orientation, direction) = self.nav.with(|n| (n.options.orientation, n.direction));
        let entry = match orientation {
            Axis::Horizontal => NamedKey::ArrowDown,
            Axis::Vertical => match direction.get() {
                gui::Direction::Ltr => NamedKey::ArrowRight,
                gui::Direction::Rtl => NamedKey::ArrowLeft,
            },
        };
        if ev.is(entry) && gui::untracked(|| self.is_open()) {
            self.enter_panel(Intent::First);
            return true;
        }
        self.nav.top_key(self.trigger, ev)
    }

    /// `d` as the trigger, usable as `availability` says. Called in render:
    /// it reads whether the panel is open (tracked). Sets the node's key,
    /// click, hover, pointer and measured handlers.
    #[must_use]
    pub fn trigger(self, d: Div, availability: crate::Availability) -> Div {
        let open = self.is_open();
        let (top, active) = self.nav.with(|n| (n.top.clone(), n.active));
        let focus = self.trigger;
        let d = self
            .link
            .trigger(top.item(d, Item::new((), focus).availability(availability)), open)
            .key(self.trigger_key())
            .aria_expanded(open);
        // The open panel is a layer (drawn in the viewport, or floating
        // from its item), read where its trigger is: inside the landmark,
        // right after the trigger, as reka's panel in its `li` (or its
        // `aria-owns` node, with a viewport) is.
        let d = if open { d.owns(self.link.content) } else { d };
        let d = d
            .on_measured(move |b| {
                self.nav.measured(focus.id(), *b);
                if gui::untracked(|| self.is_open()) && active.is_alive() && active.get_untracked() != Some(*b) {
                    active.set(Some(*b));
                }
            })
            .on_key_down(move |ev| self.on_trigger_key(ev));
        if !availability.actionable() {
            return d;
        }
        d.on_pointer_move(move |ev| self.moved(ev.x, ev.y))
            .on_hover(move |inside| self.hovered(inside))
            .on_touch(move |t| {
                use gui::events::TouchPhase;
                // `Ended` comes after the tap's click.
                self.flags(|f| {
                    f.touched
                        .set(t.phase == TouchPhase::Started || t.phase == TouchPhase::Moved)
                });
            })
            .on_click(move |_| {
                // Only the mouse's click is turned off: a tap, Enter and
                // Space (the engine presses the focused trigger) still toggle.
                let by_mouse =
                    !self.flags(|f| f.touched.get()) && gui::untracked(gui::focus::modality) == gui::Modality::Pointer;
                let disable_click = self.nav.with(|n| n.options.disable_click);
                if (by_mouse && disable_click) || self.flags(|f| f.moved_open.get()) {
                    return;
                }
                self.open_toggle();
            })
    }

    /// `d` as the panel, shown while open: a layer that is not modal and
    /// has no focus scope, named by its trigger; the pointer in it holds it
    /// open, leaving it closes it. Place it in a `gui::deferred` declared in
    /// the item.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let nav = self.nav;
        let links = self.links.get_untracked();
        let trigger = self.trigger;
        let size = nav.with(|n| n.viewport_size);
        let d = if nav.has_viewport() {
            d.on_measured(move |b| {
                let now = Some((b.width, b.height));
                if size.is_alive() && size.get_untracked() != now {
                    size.set(now);
                }
            })
        } else {
            d
        };
        self.link
            .content(d, true)
            .on_hover(move |inside| {
                if inside {
                    nav.content_enter();
                } else {
                    nav.content_leave();
                }
            })
            .on_key_down(move |ev| {
                let at = links.focused();
                let Some(intent) = Keymap::new(Arrows::Both, Wrap::Stop).intent(ev, nav.with(|n| n.direction).get())
                else {
                    return false;
                };
                let to = links.with(|list| {
                    Keymap::LINE
                        .step(list, at, intent)
                        .and_then(|i| list.get(i))
                        .map(|i| i.focus)
                });
                if let Some(f) = to {
                    gui::focus::set_focus(Some(f.id()), true);
                }
                at.is_some()
            })
            .layer(
                Layer::new()
                    .dismiss(DismissPolicy::DEFAULT)
                    .on_dismiss(move |reason| self.dismissed(reason, trigger)),
            )
    }

    fn dismissed(self, reason: &DismissReason, trigger: FocusHandle) -> Dismiss {
        match reason {
            // A nested menu's panel never closes by a press outside it.
            DismissReason::PointerOutside(_) if self.nav.is_sub() => return Dismiss::Veto,
            DismissReason::PointerOutside(p) if self.nav.on_menu(p.x, p.y) => return Dismiss::Veto,
            DismissReason::FocusOutside => {
                // Focus elsewhere in the navigation (a trigger, a link, a
                // nested menu) is not outside.
                let focused = gui::untracked(gui::focus::focused);
                if focused.is_some_and(|f| self.nav.root_menu().holds(f)) {
                    return Dismiss::Veto;
                }
            }
            DismissReason::Escape => {
                self.nav.close();
                self.flags(|f| f.escape_closed.set(true));
                gui::focus::set_focus(Some(trigger.id()), true);
                return Dismiss::Close;
            }
            _ => {}
        }
        self.nav.close();
        Dismiss::Close
    }

    /// The trigger of the root menu's item whose panel holds this one
    /// (itself in a menu that is not nested): what focus goes back to when
    /// a link picked at any depth closes the whole menu.
    fn root_trigger(self) -> FocusHandle {
        let mut item = self;
        while let Some(parent) = item.nav.with(|n| n.parent) {
            match gui::untracked(|| parent.open_item()) {
                Some(open) => item = open,
                None => break,
            }
        }
        item.trigger
    }

    /// `d` as a link in this item's panel, focused through `focus`, current
    /// when `active`; picking it runs `on_select`, whose answer closes the
    /// whole menu, nested menus included (focus going back to the root
    /// item's trigger), or keeps it.
    #[must_use]
    pub fn link(
        self,
        d: Div,
        focus: FocusHandle,
        active: bool,
        on_select: impl Into<gui::Callback<(), Dismiss>>,
    ) -> Div {
        let links = self.links.get_untracked();
        // A panel that is not open (kept while it leaves) is no Tab stop
        // (reka's `removeFromTabOrder`).
        let d = links
            .item(d, Item::new((), focus))
            .tab_stop(move || gui::untracked(|| self.is_open()));
        self.nav.link_part(
            d,
            active,
            on_select.into(),
            gui::Callback::new(move |()| {
                if links.focused().is_some() {
                    gui::focus::set_focus(Some(self.root_trigger().id()), false);
                }
            }),
        )
    }
}
