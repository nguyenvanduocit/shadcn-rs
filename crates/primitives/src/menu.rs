//! The menu engine (reka's `Menu`): what `DropdownMenu`, `ContextMenu` and
//! `Menubar` share (research §8, "Menu").
//!
//! A [`Menu`] is one level: the root or a submenu ([`Menu::submenu`]). Each has
//! its own open state, its own items, its own arrow keys and typeahead, and
//! the item it highlights. Its parts are functions that take the look's
//! `Div` and add the behaviour: [`Menu::content`], [`Menu::item`],
//! [`Menu::checkbox_item`], [`Menu::radio_item`], [`Menu::sub_trigger`],
//! [`group`], [`label`], [`separator`]. The wrappers add the trigger and
//! say how the menu opens ([`crate::dropdown_menu`], [`crate::context_menu`],
//! [`crate::menubar`]).
//!
//! # What it keeps (reka's guarantees, research §8)
//!
//! - **The highlight is focus.** The item under the mouse takes focus
//!   (without scrolling: `gui` never scrolls on focus); the mouse leaving
//!   it gives focus to the content, and then no item is highlighted, so the
//!   next arrow starts from an end. The look reads the highlight from the
//!   item's node ([`gui::Interaction::focused`], or [`Menu::is_highlighted`]
//!   in a binding): moving it renders no item.
//! - **Each level is its own.** A content navigates and searches its own
//!   items only (`Collection` per level); keys in a submenu never reach its
//!   parent's items.
//! - **Keys** (`MenuContentImpl`, `MenuSubTrigger`, `MenuSubContent`):
//!
//!   | Key | Result |
//!   | --- | --- |
//!   | ArrowDown / ArrowUp | the next / previous enabled item; from the content, the first / the last; stops at the ends unless `wrap` |
//!   | Home, PageUp / End, PageDown | the first / last enabled item |
//!   | Enter, Space | pick the highlighted item; Space is part of a running typeahead search instead |
//!   | a printable character | typeahead (1000 ms), not with Control, Alt or Command |
//!   | ArrowRight (LTR), ArrowLeft (RTL), Enter, Space on a sub trigger | open its submenu on its first item |
//!   | ArrowLeft (LTR), ArrowRight (RTL) in a submenu | close it, focus back on its trigger |
//!   | the same two keys elsewhere | the root's sideways move, if it has one (a menubar's next and previous menu) |
//!   | Escape, at any level | close the whole menu (the root's layer takes it: ADR-0024) |
//!   | Tab | modal: nothing (no item is a Tab stop); not modal: focus leaves and the menu closes |
//!
//! - **A press elsewhere released on an item picks it** (reka's
//!   `pointerup`, through [`gui::Div::on_release`]): a menubar trigger
//!   pressed, dragged to an item and released; a press on one item
//!   released on another; a context menu's long press lifted on an item.
//!   A press and release on the item itself is its click.
//! - **A filter** ([`Menu::use_filter`], [`Menu::filter_key`]; the part is
//!   [`crate::dropdown_menu::DropdownMenuFilter`]): while it holds focus,
//!   the highlight is the level's own, not focus ([`Menu::is_highlighted`]
//!   reads both), named by [`Menu::active_node`]; the pointer coming into
//!   the content focuses it.
//! - **Picking closes the whole menu** unless the item's handler answers
//!   [`Dismiss::Veto`] (reka's prevented `select` event). Checkbox and
//!   radio items close too. A checkbox item asks for the value its pick
//!   turns it to (indeterminate becomes checked, else it flips); a radio
//!   item asks for its value, even when chosen already. The caller keeps
//!   both: one that does not write the request back sees nothing change.
//! - **Opening.** The content takes focus, however it opened;
//!   opened from the keyboard, its first item does ([`OpenFocus`]). A
//!   submenu opened by the pointer leaves focus on its trigger.
//! - **Submenus.** The mouse moving on a sub trigger opens it after
//!   [`SUB_OPEN_DELAY`] (from the first move, not the entry; leaving first
//!   cancels it); a click opens it at once and never closes it. Leaving the
//!   trigger toward the open submenu draws reka's grace area: a pentagon
//!   from the exit point, moved [`GRACE_BLEED`] px away from the submenu,
//!   to the submenu's four corners on its side as placed (after a flip).
//!   For [`GRACE`] after, while the pointer stays in it moving that way,
//!   other items take no highlight and the submenu stays. A submenu closes
//!   when another item of its parent is highlighted (not when the
//!   highlight is merely gone: it may cover its trigger), when focus that
//!   was in it leaves for anything but its trigger or its parent's filter
//!   (reka's `focusout` on the submenu's content), and whenever its parent
//!   opens or closes: a parent opened again never shows a submenu open.
//! - **Dismissal** is the engine's (ADR-0024): the root's content is a
//!   [`Layer`], modal or not. Escape at any level, a press outside the
//!   whole tree (a submenu is inside its root by ownership) or, when not
//!   modal, focus outside, close the whole menu. A modal menu traps focus,
//!   blocks the page below and hides it from assistive technology; the
//!   first press outside only closes it. Focus goes back as the engine's
//!   focus scope gives it (to the trigger, reka's `onCloseAutoFocus`; with
//!   no trigger there, to what held it as the menu opened; unless something
//!   else took it since or a press outside closed it): the wrappers' rules
//!   are that one (research §8, the wrapper table). A wrapper vetoes what is not outside for it ([`Menu::veto`]).
//! - **Assistive technology.** The content is a `menu`, vertical, named by
//!   its trigger where the wrapper says so; items are `menuitem`,
//!   `menuitemcheckbox` (checked, unchecked, mixed) and `menuitemradio`;
//!   a disabled item is read, disabled. A sub trigger has `aria-haspopup=menu`
//!   and `aria-expanded`
//!   and controls its submenu only while it is open; a submenu is named by
//!   its trigger. A group is named by its label only when it has one (reka
//!   points `aria-labelledby` at a label that may not exist: research §4
//!   item 13). A separator is a horizontal `separator`.
//!
//! # Where it differs from reka, and why
//!
//! - PageUp and PageDown reach the first and last item from an item too, as
//!   everywhere (ADR-0025 §3); reka takes them on the content alone.
//! - Space during a typeahead search is part of it (ADR-0025 §6); reka's
//!   menu drops it, so "New Tab" cannot be typed.
//! - Under RTL, the key that opens a submenu is the one that moves to the
//!   next menubar menu from a submenu's plain item too (ADR-0025, research
//!   §4 item 13): reka hard-codes ArrowRight there.
//! - The pointer's direction for the grace area is read from moves over
//!   the content, as reka does (`clientX`, not `movementX`).

use gui::engine::{WorldCell, WorldRefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::focus::FocusId;
use gui::{
    AutoFocus, Axis, Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, FocusScope, HasPopup, KeyEvent, Layer,
    NamedKey, NodeRef, Placement, Role, Side, Signal, Toggled, UseModel,
};

use crate::collection::{Collection, Item};
use crate::direction::{ReadDirection, use_direction};
use crate::keymap::{Intent, Keymap, Wrap, activates};
use crate::presence::Presence;
use crate::relation::{Linked, Points};
use crate::typeahead::Typeahead;

/// How long the mouse moves on a sub trigger before its submenu opens
/// (`MenuSubTrigger.vue:73-78`).
pub const SUB_OPEN_DELAY: Duration = Duration::from_millis(100);
/// How long the pointer has to reach a submenu through the grace area
/// (`MenuSubTrigger.vue:28-29`, `graceDuration`).
pub const GRACE: Duration = Duration::from_millis(300);
/// How far the grace area's first point lies behind the exit point, away
/// from the submenu, so the exit point is inside it (`MenuSubTrigger.vue:96`).
pub const GRACE_BLEED: f32 = 5.0;

/// Whether a level is open, wherever it is kept: a signal, a part's
/// `v-model:open`, a menubar's value.
pub trait OpenModel {
    /// Tracked.
    fn get(&self) -> bool;
    /// Untracked.
    fn get_untracked(&self) -> bool {
        gui::untracked(|| self.get())
    }
    /// Ask for `open`; an equal write does nothing.
    fn set(&self, open: bool);
}

impl OpenModel for Signal<bool> {
    fn get(&self) -> bool {
        Signal::get(self)
    }

    fn get_untracked(&self) -> bool {
        Signal::get_untracked(self)
    }

    fn set(&self, open: bool) {
        if Signal::get_untracked(self) != open {
            Signal::set(self, open);
        }
    }
}

impl<P: 'static> OpenModel for UseModel<P, bool> {
    fn get(&self) -> bool {
        UseModel::get(self)
    }

    fn get_untracked(&self) -> bool {
        UseModel::get_untracked(self)
    }

    fn set(&self, open: bool) {
        UseModel::set(self, open);
    }
}

/// Where focus goes in a content as it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpenFocus {
    /// The content itself, no item highlighted: opened by the pointer.
    Content,
    /// The first enabled item: opened from the keyboard.
    First,
}

impl OpenFocus {
    /// [`Self::First`] while the user is at the keyboard, else
    /// [`Self::Content`] (reka's `isUsingKeyboard` entry focus).
    #[must_use]
    pub fn by_modality() -> Self {
        if gui::untracked(gui::focus::modality) == gui::Modality::Keyboard {
            Self::First
        } else {
            Self::Content
        }
    }
}

/// What a menu root is set up with (reka's `MenuRoot` and `MenuContent`
/// props).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuOptions {
    /// Modal (reka: true): focus kept in, the page below inert and hidden.
    pub modal: bool,
    /// Past the last item, the arrows go on from the first (reka's `loop`,
    /// off).
    pub wrap: Wrap,
    /// The content stays mounted, hidden, while closed (`forceMount`).
    pub keep_mounted: bool,
    /// The content is named by its trigger (not a context menu's).
    pub labelled: bool,
}

impl MenuOptions {
    /// reka's defaults.
    pub const DEFAULT: Self = Self {
        modal: true,
        wrap: Wrap::Stop,
        keep_mounted: false,
        labelled: true,
    };

    #[must_use]
    pub const fn modal(self, modal: bool) -> Self {
        Self { modal, ..self }
    }

    #[must_use]
    pub const fn wrap(self, wrap: Wrap) -> Self {
        Self { wrap, ..self }
    }

    #[must_use]
    pub const fn keep_mounted(self, keep_mounted: bool) -> Self {
        Self { keep_mounted, ..self }
    }

    #[must_use]
    pub const fn labelled(self, labelled: bool) -> Self {
        Self { labelled, ..self }
    }
}

impl Default for MenuOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What picking an item runs: its answer closes the menu or keeps it open.
type Pick = gui::Callback<(), Dismiss>;

/// What an item of a level is.
#[derive(Clone)]
pub struct Row {
    kind: Kind,
    /// Its box as last laid out, while a filter is there: its own, held
    /// in the level's list of the items laid out (never a table of the
    /// level's that rows would add to).
    measured: Rc<WorldCell<Option<gui::Bounds>>>,
}

#[derive(Clone)]
enum Kind {
    Item(Pick),
    Sub(Menu),
}

/// The grace area toward an open submenu (reka's `GraceIntent`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Grace {
    area: [(f32, f32); 5],
    /// The side the submenu is on: the pointer moves toward it.
    side: Side,
}

/// What the pointer was over in a move, recorded by the item before the
/// content resolves the move.
#[derive(Clone, Copy)]
struct Hit {
    focus: FocusHandle,
    /// The submenu it opens, for a sub trigger.
    sub: Option<Menu>,
    usable: bool,
}

type Sideways = gui::Callback<bool>;
type Veto = gui::Callback<DismissReason, bool>;

/// Its cells are the world's that made the level: a check writes none of a
/// live one's (ADR-0034, S7). The sideways move and veto are set only by
/// this crate's constructors (`pub(crate)`) in the setup that made the
/// level, and read by the level's own handlers.
#[derive(Clone)]
struct Level {
    open: Rc<dyn OpenModel>,
    parent: Option<Menu>,
    subs: gui::Registry<Menu>,
    items: Collection<Row>,
    typeahead: Typeahead,
    /// The content's focus: it takes focus while no item is highlighted.
    content: FocusHandle,
    /// The trigger's focus: the root's trigger, or a submenu's sub trigger.
    trigger: FocusHandle,
    link: Linked,
    direction: ReadDirection,
    options: MenuOptions,
    /// ArrowLeft and ArrowRight past the items (a menubar's menus).
    sideways: WorldRefCell<Option<Sideways>>,
    /// What a wrapper does not count as outside.
    veto: WorldRefCell<Option<Veto>>,
    /// Where this level's content was placed (a submenu's side, for the
    /// grace area; the look's side and origin).
    placed: Signal<Option<Placement>>,
    /// The pointer over this content: where it last moved, which way along
    /// x, and what it was over.
    pointer: WorldCell<(f32, f32)>,
    heading: WorldCell<Option<Side>>,
    hit: WorldCell<Option<Hit>>,
    over: WorldCell<Option<FocusId>>,
    /// The pointer moved over this content since it opened: only then does
    /// its leaving take the highlight away.
    inside: WorldCell<bool>,
    grace: WorldCell<Option<Grace>>,
    grace_timer: WorldCell<Option<TimerId>>,
    /// A submenu's opening on the mouse's rest.
    open_timer: WorldCell<Option<TimerId>>,
    /// The search field in this content (`DropdownMenuFilter`), if any:
    /// while it holds focus the highlight is [`Level::active`], not focus.
    filter: gui::Holder<FocusHandle>,
    /// The highlighted item, written with every highlight; what is
    /// highlighted while the filter keeps focus (reka's
    /// `highlightedElement`).
    active: Signal<Option<FocusId>>,
    /// The node the highlighted item carries while a filter is there: the
    /// filter's active descendant (one node, whichever item it is on).
    active_ref: NodeRef,
    /// The content's scroll container and, while a filter is there, the
    /// items' boxes: a highlight the filter's keys move is scrolled into
    /// view (`scrollIntoView({ block: 'nearest' })`).
    scroll: gui::Holder<gui::ScrollHandle>,
}

/// One level of a menu, the root or a submenu: made once in its part's
/// setup, `Copy`.
#[derive(Clone, Copy)]
pub struct Menu {
    level: Signal<Level>,
}

impl PartialEq for Menu {
    fn eq(&self, other: &Self) -> bool {
        self.level == other.level
    }
}

impl std::fmt::Debug for Menu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Menu").finish_non_exhaustive()
    }
}

fn cancel(timer: &WorldCell<Option<TimerId>>) {
    if let Some(t) = timer.take() {
        clock::cancel(t);
    }
}

/// Whether `p` is inside `polygon` (reka's `isPointInPolygon`, ray casting).
fn in_polygon(p: (f32, f32), polygon: &[(f32, f32)]) -> bool {
    let (x, y) = p;
    let mut inside = false;
    let last = polygon.last().copied();
    let mut j = match last {
        Some(l) => l,
        None => return false,
    };
    for &(xi, yi) in polygon {
        let (xj, yj) = j;
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = (xi, yi);
    }
    inside
}

impl Menu {
    /// A root level open as `open` says, set up as `options`; called in the
    /// root part's setup. Its direction is the nearest provided one
    /// ([`crate::use_direction`]).
    #[must_use]
    #[track_caller]
    pub fn new(open: impl OpenModel + 'static, options: MenuOptions) -> Self {
        let me = Self::level(Rc::new(open), None, options);
        // Opened by its caller, the content takes focus as reka's does on
        // mount; opened by `open`, focus is already where it says. The
        // layer's scope moves none (`AutoFocus::Keep`), so it never undoes
        // the step to the first item.
        gui::watch(
            move || me.is_open(),
            move |open, _| {
                if *open && me.level.is_alive() && !me.holds_focus() {
                    gui::focus::set_focus(Some(me.content_focus().id()), false);
                }
            },
        );
        if me.is_open_untracked() {
            gui::after_layout(move || {
                if me.level.is_alive() && me.is_open_untracked() && !me.holds_focus() {
                    gui::focus::set_focus(Some(me.content_focus().id()), false);
                }
            });
        }
        me
    }

    #[track_caller]
    fn level(open: Rc<dyn OpenModel>, parent: Option<Menu>, options: MenuOptions) -> Self {
        Self {
            level: Signal::new(Level {
                open,
                parent,
                subs: gui::Registry::new(),
                items: Collection::new(),
                typeahead: Typeahead::new(),
                content: FocusHandle::new(),
                trigger: FocusHandle::new(),
                link: Linked::new(Points::Controls),
                direction: use_direction(None),
                options,
                sideways: WorldRefCell::new("a menu level's sideways move", None),
                veto: WorldRefCell::new("a menu level's veto", None),
                placed: Signal::labeled("menu placed", None),
                pointer: WorldCell::new("a menu level's pointer", (0.0, 0.0)),
                heading: WorldCell::new("a menu level's pointer heading", None),
                hit: WorldCell::new("a menu level's hit", None),
                over: WorldCell::new("the item a menu level's pointer is over", None),
                inside: WorldCell::new("whether the pointer moved in a menu level", false),
                grace: WorldCell::new("a menu level's grace area", None),
                grace_timer: WorldCell::new("a menu level's grace timer", None),
                open_timer: WorldCell::new("a submenu's open timer", None),
                filter: gui::Holder::new(),
                active: Signal::labeled("menu highlight", None),
                active_ref: NodeRef::new(),
                scroll: gui::Holder::new(),
            }),
        }
    }

    /// A submenu of this level, open as `open` says (reka's `MenuSub`);
    /// called in the sub part's setup, it leaves this level as the part
    /// unmounts. It closes when focus that was in it leaves for anything
    /// but its trigger or its parent's filter.
    #[must_use]
    #[track_caller]
    pub fn submenu(self, open: impl OpenModel + 'static) -> Self {
        let options = self.with(|l| l.options).modal(false);
        let sub = Self::level(Rc::new(open), Some(self), options);
        self.with(|l| l.subs).register(sub);
        gui::on_cleanup(move || {
            if sub.level.is_alive() {
                sub.with(|l| {
                    cancel(&l.open_timer);
                    cancel(&l.grace_timer);
                });
            }
        });
        // Its parent opening or closing closes it (reka's `MenuSub`).
        gui::watch(
            move || self.is_open(),
            move |_, _| {
                if sub.level.is_alive() && sub.is_open_untracked() {
                    sub.close();
                }
            },
        );
        // Focus leaving the submenu closes it (reka's `focusout` on the
        // submenu's content): only focus that was in it, so the highlight
        // merely going away (focus back on the parent's content) keeps it
        // open, as it may cover its trigger.
        gui::watch(gui::focus::focused, move |now, was| {
            let (now, Some(was)) = (*now, *was) else { return };
            if !sub.level.is_alive() || !sub.with(|l| l.open.get_untracked()) || !sub.holds(was) {
                return;
            }
            let to_filter = self.level.is_alive() && self.with(|l| l.filter.get()).is_some_and(|f| Some(f.id()) == now);
            if now != Some(sub.trigger_focus().id()) && !now.is_some_and(|n| sub.holds(n)) && !to_filter {
                sub.close();
            }
        });
        sub
    }

    fn with<R>(self, f: impl FnOnce(&Level) -> R) -> R {
        self.level.with_untracked(f)
    }

    /// Run `f` (a menubar's move to its next menu, `true`, or previous one)
    /// for the arrows past this root's items: ArrowRight and ArrowLeft, as
    /// the direction turns them.
    pub(crate) fn set_sideways(self, f: impl Into<gui::Callback<bool>>) {
        let f = f.into();
        self.with(|l| l.sideways.with_mut(|s| *s = Some(f)));
    }

    /// Refuse the dismiss requests `veto` says are not outside for this
    /// wrapper (a context menu's right click in its area, a menubar's focus
    /// on its triggers).
    pub(crate) fn veto(self, veto: impl Into<gui::Callback<DismissReason, bool>>) {
        let veto = veto.into();
        self.with(|l| l.veto.with_mut(|v| *v = Some(veto)));
    }

    /// Whether this level is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        // A binding of a node on its way out may read a level whose scope
        // has ended: it is closed.
        if !self.level.is_alive() {
            return false;
        }
        let open = self.with(|l| Rc::clone(&l.open));
        open.get()
    }

    /// Whether this level is open, untracked (an event handler's).
    #[must_use]
    pub fn is_open_untracked(self) -> bool {
        if !self.level.is_alive() {
            return false;
        }
        let open = self.with(|l| Rc::clone(&l.open));
        open.get_untracked()
    }

    /// Where its content is (tracked): open, kept mounted while closed, or
    /// gone.
    #[must_use]
    pub fn presence(self) -> Presence {
        Presence::of(self.is_open(), self.with(|l| l.options.keep_mounted))
    }

    /// The level above, for a submenu.
    #[must_use]
    pub fn parent(self) -> Option<Self> {
        self.with(|l| l.parent)
    }

    /// The root of this level's menu.
    #[must_use]
    pub fn root(self) -> Self {
        let mut at = self;
        while let Some(p) = at.parent() {
            at = p;
        }
        at
    }

    /// The focus the trigger takes: give it to the trigger's node (a
    /// submenu's: its sub trigger, which [`Menu::sub_trigger`] does).
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.with(|l| l.trigger)
    }

    /// The content's own focus.
    #[must_use]
    pub fn content_focus(self) -> FocusHandle {
        self.with(|l| l.content)
    }

    /// The trigger and the content as assistive technology links them.
    #[must_use]
    pub fn link(self) -> Linked {
        self.with(|l| l.link)
    }

    /// Where layout placed this level's content: give it to the content's
    /// `gui::deferred(…).placement`; the look reads the side and the room.
    #[must_use]
    pub fn placed(self) -> Signal<Option<Placement>> {
        self.with(|l| l.placed)
    }

    /// The side a submenu opens on: the inline end (right, or left under
    /// RTL), read when asked (reka's `MenuSubContent` side).
    #[must_use]
    pub fn sub_side(self) -> Side {
        match self.with(|l| l.direction).get() {
            gui::Direction::Ltr => Side::Right,
            gui::Direction::Rtl => Side::Left,
        }
    }

    /// Whether the item focused through `focus` is highlighted (tracked):
    /// it holds focus. A look binds it to the item's node.
    #[must_use]
    pub fn is_highlighted(self, focus: FocusHandle) -> bool {
        let now = gui::focus::focused();
        if now == Some(focus.id()) {
            return true;
        }
        // While a filter keeps focus, the highlight is the level's own.
        self.level.is_alive()
            && self.with(|l| l.filter).get().is_some_and(|f| Some(f.id()) == now)
            && self.with(|l| l.active).get() == Some(focus.id())
    }

    /// Whether this level's filter holds focus now (untracked).
    fn filter_focused(self) -> bool {
        self.with(|l| l.filter.get_untracked())
            .is_some_and(|f| gui::untracked(gui::focus::focused) == Some(f.id()))
    }

    /// Register `field` as this level's filter (a search field in its
    /// content: `DropdownMenuFilter`), until the calling scope ends; called
    /// in the filter part's setup.
    #[track_caller]
    pub fn use_filter(self, field: FocusHandle) {
        // Held from its mount to its unmount, by its registration: a
        // filter that replaced it (the same `field` or not) keeps it.
        self.with(|l| l.filter).hold(field);
    }

    /// The highlighted item's node (tracked): a filter's
    /// `aria-activedescendant`.
    #[must_use]
    pub fn active_node(self) -> Option<NodeRef> {
        if !self.level.is_alive() {
            return None;
        }
        self.with(|l| l.active).get()?;
        Some(self.with(|l| l.active_ref))
    }

    /// How many rows this level keeps anything of (their boxes, their
    /// nodes): those laid out, filled by the layout; none beyond it.
    #[must_use]
    pub fn kept_rows(self) -> usize {
        self.with(|l| l.items.len())
    }

    /// The keys a filter of this level takes before its own editing
    /// (`DropdownMenuFilter`): the vertical arrows, Home and End move the
    /// highlight (focus stays in the field), Enter picks it, Escape empties
    /// a field that holds text (the menu stays). While an input method
    /// composes, every key is its own.
    pub fn filter_key(self, ev: &KeyEvent, field: &gui::TextField) -> bool {
        if field.edit.with_untracked(|e| e.preedit().is_some()) {
            return false;
        }
        let m = ev.modifiers;
        if m.control || m.alt || m.command {
            return false;
        }
        let current = self.current();
        let intent = match &ev.key {
            gui::events::Key::Named(NamedKey::ArrowDown) => Some(Intent::Next),
            gui::events::Key::Named(NamedKey::ArrowUp) => Some(Intent::Prev),
            gui::events::Key::Named(NamedKey::Home) => Some(Intent::First),
            gui::events::Key::Named(NamedKey::End) => Some(Intent::Last),
            _ => None,
        };
        if let Some(intent) = intent {
            self.step(intent, current);
            self.reveal_active();
            return true;
        }
        if ev.is(NamedKey::Enter) {
            let items = self.with(|l| l.items.clone());
            if let Some(r) = items.with(|list| current.and_then(|i| list.get(i)).map(|i| i.value.clone())) {
                self.pick(&r, true);
            }
            return true;
        }
        if ev.is(NamedKey::Escape) && field.edit.with_untracked(|e| !e.text().is_empty()) {
            field.set_text("");
            return true;
        }
        false
    }

    /// The content scrolls in `handle` (the look's `track_scroll`) while
    /// the calling setup's part is mounted: a highlight moved from a filter
    /// is brought into view in it.
    #[track_caller]
    pub fn scroll_with(self, handle: gui::ScrollHandle) {
        self.with(|l| l.scroll).hold(handle);
    }

    /// Scroll the highlighted item into the content's view, the nearest way.
    fn reveal_active(self) {
        let (scroll, active) = self.with(|l| (l.scroll.get_untracked(), l.active.get_untracked()));
        let (Some(scroll), Some(active)) = (scroll, active) else {
            return;
        };
        let items = self.with(|l| l.items.clone());
        let b = items.with(|list| {
            list.iter()
                .find(|i| i.focus.id() == active)
                .and_then(|i| i.value.measured.get())
        });
        let Some(b) = b else {
            return;
        };
        let Some(layout) = gui::untracked(|| scroll.layout()) else {
            return;
        };
        let top = layout.bounds.y + layout.padding.top;
        let bottom = layout.bounds.bottom() - layout.padding.bottom;
        let (x, y) = layout.offset;
        let to = if b.y < top {
            y - (top - b.y)
        } else if b.bottom() > bottom {
            y + (b.bottom() - bottom)
        } else {
            return;
        };
        scroll.scroll_to((x, to.clamp(0.0, layout.max.1)));
    }

    /// The highlighted item's index: the focused one, or, while the filter
    /// keeps focus, the level's highlight.
    fn current(self) -> Option<usize> {
        let (items, active) = self.with(|l| (l.items.clone(), l.active));
        let focused = gui::untracked(gui::focus::focused)?;
        items.index_of(focused).or_else(|| {
            if self.filter_focused() {
                active.get_untracked().and_then(|a| items.index_of(a))
            } else {
                None
            }
        })
    }

    /// Open this level, focus going as `focus` says.
    pub fn open(self, focus: OpenFocus) {
        self.show();
        let content = self.content_focus();
        match focus {
            OpenFocus::Content => {
                if !content.is_focused() {
                    gui::focus::set_focus(Some(content.id()), false);
                }
            }
            OpenFocus::First => {
                gui::focus::set_focus(Some(content.id()), true);
                // Its items join its list as they are laid out: step once they are.
                gui::after_layout(move || {
                    if self.level.is_alive() && self.is_open_untracked() {
                        self.step(Intent::First, None);
                    }
                });
            }
        }
    }

    /// Open this level, focus left where it is (a submenu opened by the
    /// pointer keeps it on its trigger).
    fn show(self) {
        self.close_subs(None);
        // The pointer resting where it is as the content opens is no move.
        if let Some(at) = gui::untracked(gui::interact::pointer_position) {
            self.with(|l| l.pointer.set(at));
        }
        let open = self.with(|l| Rc::clone(&l.open));
        open.set(true);
    }

    /// Open this submenu by the pointer: focus stays on (or comes to) its
    /// trigger.
    fn show_from_trigger(self) {
        self.show();
        let trigger = self.trigger_focus();
        if !trigger.is_focused() {
            gui::focus::set_focus(Some(trigger.id()), false);
        }
    }

    /// Close this level and every submenu under it; it opens afresh next
    /// time: no highlight, an empty search.
    pub fn close(self) {
        self.close_subs(None);
        let open = self.with(|l| {
            cancel(&l.open_timer);
            cancel(&l.grace_timer);
            l.grace.set(None);
            l.over.set(None);
            l.inside.set(false);
            l.typeahead.reset();
            if l.active.get_untracked().is_some() {
                l.active.set(None);
            }
            Rc::clone(&l.open)
        });
        open.set(false);
    }

    /// Close the whole menu, from any level.
    pub fn close_all(self) {
        self.root().close();
    }

    /// Close this level's submenus, but the one whose trigger is `keep`.
    fn close_subs(self, keep: Option<FocusId>) {
        self.subs(|sub| {
            if Some(sub.trigger_focus().id()) != keep && sub.is_open_untracked() {
                sub.close();
            }
            true
        });
    }

    /// Walk this level's submenus in order while `f` says go on (closing
    /// one may run code that reads the list or ends a submenu: the walk
    /// still reaches the next), allocating nothing, a few links each.
    fn subs(self, f: impl FnMut(Self) -> bool) {
        if self.level.is_alive() {
            self.with(|l| l.subs).walk(f);
        }
    }

    /// The links this level's list of submenus has stepped through: the
    /// measure of the work walking it costs.
    #[must_use]
    pub fn sub_visits(self) -> u64 {
        if !self.level.is_alive() {
            return 0;
        }
        self.with(|l| l.subs).visits()
    }

    /// Whether `focus` is this level's content, one of its items, or in one
    /// of its submenus.
    #[must_use]
    pub fn holds(self, focus: FocusId) -> bool {
        if !self.level.is_alive() {
            return false;
        }
        let (content, items) = self.with(|l| (l.content, l.items.clone()));
        if content.id() == focus || items.index_of(focus).is_some() {
            return true;
        }
        let mut found = false;
        self.subs(|s| {
            found = s.holds(focus);
            !found
        });
        found
    }

    /// Whether focus is anywhere in the menu now.
    #[must_use]
    pub fn holds_focus(self) -> bool {
        gui::untracked(gui::focus::focused).is_some_and(|f| self.root().holds(f))
    }

    /// Highlight the item focused through `focus`: it takes focus, and the
    /// submenus of this level close but its own.
    fn highlight(self, focus: FocusHandle, by_keyboard: bool) {
        self.close_subs(Some(focus.id()));
        let active = self.with(|l| l.active);
        if active.get_untracked() != Some(focus.id()) {
            active.set(Some(focus.id()));
        }
        // A filter holding focus keeps it (reka: an input focused in the
        // menu is not left for an item).
        if self.filter_focused() {
            return;
        }
        if !focus.is_focused() || by_keyboard {
            gui::focus::set_focus(Some(focus.id()), by_keyboard);
        }
    }

    /// Move the highlight as `intent` says from item `from` (none: the
    /// content holds focus).
    fn step(self, intent: Intent, from: Option<usize>) {
        let (items, wrap) = self.with(|l| (l.items.clone(), l.options.wrap));
        let keymap = Keymap::LIST.wrap(wrap);
        let to = items.with(|list| {
            keymap
                .step(list, from, intent)
                .and_then(|i| list.get(i))
                .map(|i| i.focus)
        });
        if let Some(focus) = to {
            self.highlight(focus, true);
        }
    }

    /// Pick item `index`: run it, then close the whole menu unless it says
    /// [`Dismiss::Veto`]; a sub trigger opens its submenu instead.
    fn pick(self, row: &Row, by_keyboard: bool) {
        match &row.kind {
            Kind::Item(f) => {
                // An item whose owner is gone answers as an item with no
                // answer: it closes.
                if f.call(()).unwrap_or_default() == Dismiss::Close && self.level.is_alive() {
                    self.close_all();
                }
            }
            Kind::Sub(sub) if by_keyboard => sub.open(OpenFocus::First),
            Kind::Sub(sub) => sub.show_from_trigger(),
        }
    }

    /// The keys a content takes, focus on it or on one of its items.
    fn key(self, ev: &KeyEvent) -> bool {
        let Some(focused) = gui::untracked(gui::focus::focused) else {
            return false;
        };
        let (content, items, typeahead, direction, wrap, parent) = self.with(|l| {
            (
                l.content,
                l.items.clone(),
                l.typeahead,
                l.direction,
                l.options.wrap,
                l.parent,
            )
        });
        let current = items.index_of(focused);
        if current.is_none() && focused != content.id() {
            return false;
        }
        let row = |i: Option<usize>| items.with(|list| i.and_then(|i| list.get(i)).map(|i| i.value.clone()));
        let direction = direction.get();
        if let Some(intent) = Keymap::LIST.wrap(wrap).intent(ev, direction) {
            self.step(intent, current);
            return true;
        }
        let m = ev.modifiers;
        let plain = !(m.control || m.alt || m.command || m.shift);
        let (inward, outward) = match direction {
            gui::Direction::Ltr => (NamedKey::ArrowRight, NamedKey::ArrowLeft),
            gui::Direction::Rtl => (NamedKey::ArrowLeft, NamedKey::ArrowRight),
        };
        if plain && ev.is(inward) {
            if let Some(Row {
                kind: Kind::Sub(sub), ..
            }) = row(current)
            {
                sub.open(OpenFocus::First);
                return true;
            }
            return self.root().go_sideways(true);
        }
        if plain && ev.is(outward) {
            if let Some(parent) = parent {
                let trigger = self.trigger_focus();
                self.close();
                // A parent with a filter takes focus back in it, its sub
                // trigger highlighted (`MenuSubContent.vue:100-105`).
                match parent.with(|l| l.filter.get_untracked()) {
                    Some(filter) => {
                        parent.with(|l| l.active).set(Some(trigger.id()));
                        gui::focus::set_focus(Some(filter.id()), true);
                    }
                    None => gui::focus::set_focus(Some(trigger.id()), true),
                }
                return true;
            }
            return self.go_sideways(false);
        }
        if let Some(typed) = typeahead.typed(ev) {
            let found = items.with(|list| {
                typeahead
                    .find(typed, list, current)
                    .and_then(|i| list.get(i))
                    .map(|i| i.focus)
            });
            if let Some(focus) = found {
                self.highlight(focus, true);
            }
            return true;
        }
        if activates(ev) {
            if let Some(r) = row(current) {
                self.pick(&r, true);
            }
            return true;
        }
        // No item is a Tab stop; a modal menu keeps Tab from the page.
        false
    }

    fn go_sideways(self, forward: bool) -> bool {
        let f = self.with(|l| l.sideways.with(Clone::clone));
        f.is_some_and(|f| {
            f.call(forward);
            true
        })
    }

    /// The dismiss request `reason` to the root's layer.
    fn dismissed(self, reason: &DismissReason) -> Dismiss {
        let veto = self.with(|l| l.veto.with(Clone::clone));
        if veto.is_some_and(|v| v.call(*reason) == Some(true)) {
            return Dismiss::Veto;
        }
        self.close_all();
        Dismiss::Close
    }

    /// The pointer moved over this content to `(x, y)`, over the item its
    /// node recorded (if any): reka's `onItemEnter`, `onItemLeave` and
    /// `onTriggerLeave`, with the move's own position.
    fn pointer(self, x: f32, y: f32) {
        let (hit, last) = self.with(|l| (l.hit.take(), l.pointer.replace((x, y))));
        let now = hit.map(|h| h.focus.id());
        // A move the engine sends again after a frame, the pointer still,
        // is no move (a browser sends no `pointermove` then): the keys'
        // highlight stays, in a content that opened under the pointer too.
        if (x, y) == last {
            return;
        }
        // The pointer coming into a content with a filter focuses it
        // (`MenuContentImpl.vue:343-350`).
        if !self.with(|l| l.inside.replace(true))
            && let Some(filter) = self.with(|l| l.filter.get_untracked())
            && !filter.is_focused()
        {
            gui::focus::set_focus(Some(filter.id()), false);
        }
        if (x - last.0).abs() > f32::EPSILON {
            let heading = if x > last.0 { Side::Right } else { Side::Left };
            self.with(|l| l.heading.set(Some(heading)));
        }
        let was = self.with(|l| l.over.replace(now));
        if was.is_some() && was != now {
            self.left(was, x, y);
        }
        let toward = self.moving_to_sub(x, y);
        match hit {
            Some(h) if h.usable => {
                if toward {
                    return;
                }
                self.highlight(h.focus, false);
                if let Some(sub) = h.sub {
                    sub.rest();
                }
            }
            _ => {
                if !toward {
                    self.unhighlight();
                }
            }
        }
    }

    /// The pointer left the content altogether.
    fn pointer_out(self) {
        if !self.with(|l| l.inside.replace(false)) {
            return;
        }
        let ((x, y), was) = self.with(|l| (l.pointer.get(), l.over.replace(None)));
        self.left(was, x, y);
        if !self.moving_to_sub(x, y) {
            self.unhighlight();
        }
    }

    /// The pointer left the item `was` at `(x, y)`: a sub trigger's opening
    /// is cancelled, and toward its open submenu the grace area begins.
    fn left(self, was: Option<FocusId>, x: f32, y: f32) {
        let Some(was) = was else { return };
        let mut found = None;
        self.subs(|s| {
            if s.trigger_focus().id() == was {
                found = Some(s);
            }
            found.is_none()
        });
        let Some(sub) = found else {
            return;
        };
        sub.with(|l| cancel(&l.open_timer));
        let placed = sub.with(|l| l.placed.get_untracked());
        let grace = placed
            .filter(|p| sub.is_open_untracked() && p.bounds.width > 0.0)
            .map(|p| {
                let b = p.bounds;
                let right = p.side != Side::Left;
                let bleed = if right { -GRACE_BLEED } else { GRACE_BLEED };
                let (near, far) = if right { (b.x, b.right()) } else { (b.right(), b.x) };
                Grace {
                    area: [
                        (x + bleed, y),
                        (near, b.y),
                        (far, b.y),
                        (far, b.bottom()),
                        (near, b.bottom()),
                    ],
                    side: if right { Side::Right } else { Side::Left },
                }
            });
        self.with(|l| {
            cancel(&l.grace_timer);
            l.grace.set(grace);
        });
        if grace.is_some() {
            // Owned by nothing: a scope that happens to run the event (an
            // item re-rendered by its highlight) must not cancel it.
            let timer = gui::detached(|| {
                clock::after(GRACE, move || {
                    if self.level.is_alive() {
                        self.with(|l| {
                            l.grace.set(None);
                            l.grace_timer.set(None);
                        });
                    }
                })
            });
            self.with(|l| l.grace_timer.set(Some(timer)));
        }
    }

    /// Whether the pointer at `(x, y)` is in the grace area, moving toward
    /// its submenu.
    fn moving_to_sub(self, x: f32, y: f32) -> bool {
        self.with(|l| {
            l.grace
                .get()
                .is_some_and(|g| l.heading.get() == Some(g.side) && in_polygon((x, y), &g.area))
        })
    }

    /// No item is highlighted: the content takes focus, unless focus is
    /// elsewhere than on one of its items (a field in it keeps it).
    fn unhighlight(self) {
        let (content, items, active) = self.with(|l| (l.content, l.items.clone(), l.active));
        if active.get_untracked().is_some() {
            active.set(None);
        }
        if items.focused().is_some() {
            gui::focus::set_focus(Some(content.id()), false);
        }
    }

    /// The mouse rests on this submenu's trigger: it opens after
    /// [`SUB_OPEN_DELAY`], unless open or already on its way.
    fn rest(self) {
        let waiting = self.with(|l| {
            let t = l.open_timer.take();
            l.open_timer.set(t);
            t.is_some()
        });
        if waiting || self.is_open_untracked() {
            return;
        }
        if let Some(parent) = self.parent() {
            parent.with(|l| l.grace.set(None));
        }
        // Owned by nothing: a scope that happens to run the event (an
        // item re-rendered by its highlight) must not cancel it.
        let timer = gui::detached(|| {
            clock::after(SUB_OPEN_DELAY, move || {
                if self.level.is_alive() {
                    self.with(|l| l.open_timer.set(None));
                    self.show_from_trigger();
                }
            })
        });
        self.with(|l| l.open_timer.set(Some(timer)));
    }

    /// `d` as this level's content: a vertical `menu` taking focus (no Tab
    /// stop), this level's keys and the pointer's highlight; the root's is
    /// the menu's [`Layer`] (ADR-0024), modal as its options say, giving
    /// focus to itself as it opens. Place it in a `gui::deferred` whose
    /// placement is [`Menu::placed`]. Called in render, it reads whether the
    /// level is open (tracked): a content kept mounted while closed
    /// ([`MenuOptions::keep_mounted`]) is hidden from assistive technology
    /// and is no layer, takes no keys and traps nothing, so it opens afresh.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let (focus, link, labelled, parent, modal, trigger) = self.with(|l| {
            (
                l.content,
                l.link,
                l.options.labelled,
                l.parent,
                l.options.modal,
                l.trigger,
            )
        });
        if !self.is_open() {
            // Kept mounted while closed: hidden, no layer, no keys, no trap.
            return d.role(Role::Menu).aria_hidden(true);
        }
        let d = link
            .content(d, labelled || parent.is_some())
            .role(Role::Menu)
            .aria_orientation(Axis::Vertical)
            .track_focus(focus)
            .tab_stop(false)
            .on_key_down(move |ev| self.key(ev))
            .on_pointer_move(move |ev| self.pointer(ev.x, ev.y))
            .on_hover(move |inside| {
                if !inside {
                    self.pointer_out();
                }
            });
        if parent.is_some() {
            return d;
        }
        d.layer(
            Layer::new()
                .modal(modal)
                .dismiss(DismissPolicy::DEFAULT.focus_outside(!modal))
                .focus_scope(
                    FocusScope::new()
                        .trap(modal)
                        .cycle(false)
                        .auto_focus(AutoFocus::Keep)
                        .return_to(trigger),
                )
                .on_dismiss(move |r| self.dismissed(r)),
        )
    }

    /// `d` as an item of this level: it joins the level's list as it is laid
    /// out (`item`'s label is what typeahead matches), is a `menuitem`, takes
    /// focus as the highlight (never a Tab stop), and picking it (a click,
    /// Enter, Space) runs `on_select`, whose answer closes the whole menu or
    /// keeps it open.
    #[must_use]
    pub fn item(self, d: Div, item: Item<()>, on_select: impl Into<gui::Callback<(), Dismiss>>) -> Div {
        self.entry(d.role(Role::MenuItem), item, Kind::Item(on_select.into()))
    }

    /// `d` as a checkbox item, `checked` now (the caller keeps it): picking
    /// it asks `on_select` for the value it turns to (mixed becomes checked,
    /// else it flips), and closes the menu unless it says otherwise.
    #[must_use]
    pub fn checkbox_item(
        self,
        d: Div,
        item: Item<()>,
        checked: Toggled,
        on_select: impl Into<gui::Callback<bool, Dismiss>>,
    ) -> Div {
        let next = !matches!(checked, Toggled::True);
        let on_select = on_select.into();
        self.entry(
            d.role(Role::MenuItemCheckBox).aria_checked(checked),
            item,
            Kind::Item(gui::Callback::new(move |()| on_select.call(next).unwrap_or_default())),
        )
    }

    /// `d` as a radio item, chosen when `checked`: picking it asks
    /// `on_select` for its value (chosen already or not; a radio item is
    /// never unchosen by a pick) and closes the menu unless it says
    /// otherwise.
    #[must_use]
    pub fn radio_item(
        self,
        d: Div,
        item: Item<()>,
        checked: bool,
        on_select: impl Into<gui::Callback<(), Dismiss>>,
    ) -> Div {
        self.entry(
            d.role(Role::MenuItemRadio).aria_checked(checked),
            item,
            Kind::Item(on_select.into()),
        )
    }

    /// `d` as the trigger of this submenu, in its parent's content: an item
    /// of the parent focused through [`Menu::trigger_focus`], with
    /// `aria-haspopup` and `aria-expanded`, controlling the submenu while it
    /// is open. The mouse resting on it opens it after [`SUB_OPEN_DELAY`],
    /// a click opens it at once (never closes it), and the inline-end arrow,
    /// Enter or Space open it on its first item. Called in render, it reads
    /// whether the submenu is open (tracked).
    #[must_use]
    pub fn sub_trigger(self, d: Div, label: impl Into<Rc<str>>, availability: crate::Availability) -> Div {
        let Some(parent) = self.parent() else {
            return d;
        };
        let open = self.is_open();
        let item = Item::new((), self.trigger_focus())
            .label(label)
            .availability(availability);
        let d = self
            .link()
            .trigger(d, open)
            .role(Role::MenuItem)
            .aria_has_popup(HasPopup::Menu)
            .aria_expanded(open);
        parent.entry(d, item, Kind::Sub(self))
    }

    fn entry(self, d: Div, item: Item<()>, kind: Kind) -> Div {
        let (focus, availability) = (item.focus, item.availability);
        let sub = match &kind {
            Kind::Sub(s) => Some(*s),
            Kind::Item(_) => None,
        };
        let row = Row {
            kind,
            measured: Rc::new(WorldCell::new("a menu row's box", None)),
        };
        let items = self.with(|l| l.items.clone());
        let item = Item {
            value: row.clone(),
            focus,
            availability,
            label: item.label,
            active: false,
        };
        let usable = availability.actionable();
        // With a filter, the item's node is what the filter names as its
        // active descendant (tracked: a filter registering renders it once).
        let d = if self.with(|l| l.filter).get().is_some() {
            let measured = Rc::clone(&row.measured);
            let d = d.on_measured(move |b| measured.set(Some(*b)));
            if self.is_highlighted(focus) {
                d.node_ref(self.with(|l| l.active_ref))
            } else {
                d
            }
        } else {
            d
        };
        let released = row.clone();
        items
            .item(d, item)
            .tab_stop(false)
            // A press elsewhere released on it picks it (reka's
            // `pointerup`): a menubar trigger pressed and dragged to it.
            .on_release(move |_| {
                if usable {
                    self.pick(&released, false);
                }
            })
            .on_pointer_move(move |_| {
                self.with(|l| l.hit.set(Some(Hit { focus, sub, usable })));
            })
            .on_click(move |_| {
                if usable {
                    self.pick(&row, false);
                }
            })
    }
}

/// `d` as a group of items (`role=group`), named by its label when it has
/// one: `label` is the [`label`] node inside it. reka names a group by a
/// label that may not exist; here a group without one is unnamed.
#[must_use]
pub fn group(d: Div, label: Option<NodeRef>) -> Div {
    let d = d.role(Role::Group);
    match label {
        Some(l) => d.labelled_by(l),
        None => d,
    }
}

/// `d` as a group's label: plain text, never focused, passed by the keys
/// and typeahead; `node` is what its group is named by.
#[must_use]
pub fn label(d: Div, node: NodeRef) -> Div {
    d.node_ref(node)
}

/// `d` as a separator between items: a horizontal `separator`, never
/// focused (reka's `MenuSeparator`).
#[must_use]
pub fn separator(d: Div) -> Div {
    crate::separator::Separator::new(Axis::Horizontal, false).apply(d)
}

/// Whether a checkbox or radio item's indicator shows: while it is checked
/// or mixed, or always when `force_mount` (reka's `MenuItemIndicator`).
#[must_use]
pub fn indicator_shown(checked: Toggled, force_mount: bool) -> bool {
    force_mount || !matches!(checked, Toggled::False)
}

#[cfg(test)]
mod tests {
    use super::in_polygon;

    #[test]
    fn the_grace_pentagon_holds_its_exit_point() {
        // Exit at (100, 50) toward a submenu at x 110..210, y 40..240.
        let area = [
            (95.0, 50.0),
            (110.0, 40.0),
            (210.0, 40.0),
            (210.0, 240.0),
            (110.0, 240.0),
        ];
        assert!(in_polygon((100.0, 50.0), &area), "the exit point is inside");
        assert!(in_polygon((105.0, 60.0), &area), "on the way down to the submenu");
        assert!(
            !in_polygon((100.0, 200.0), &area),
            "far below the exit, beside the submenu"
        );
        assert!(!in_polygon((90.0, 50.0), &area), "behind the bleed");
        assert!(!in_polygon((0.0, 0.0), &[]), "no area");
    }
}
