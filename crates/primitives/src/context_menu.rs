//! A menu opened in an area, at the pointer (reka's `ContextMenu`; research
//! §8, "ContextMenu"), on the [`Menu`] engine.
//!
//! - **The area** (`ContextMenuTrigger`) has no role and takes no focus of
//!   its own. A secondary press in it opens the menu at the pointer (the
//!   platform's menu does not show: `gui` has none); a touch or pen held
//!   [`ContextMenu::press_open_delay`] ([`gui::gesture::LONG_PRESS`],
//!   700 ms, unless the caller sets another, reka's `pressOpenDelay`)
//!   without moving past [`gui::gesture::SLOP`] does too (the engine's long
//!   press, ADR-0024); lifting that contact over an item picks it.
//!   The context-menu key, or Shift+F10, with focus inside the area opens
//!   it on its first item against the whole area: what a browser sends as
//!   `contextmenu` from the keyboard, which reka listens to.
//! - **Anchor.** Opened by the pointer, the menu stands at the point;
//!   opened by its caller (`v-model:open`), against the whole area. The
//!   point is kept while it closes, so it does not move during its exit.
//! - A secondary press in the area while it is open moves the menu to the
//!   new point; it does not close and open again (reka's test). It is no
//!   press outside: the content's layer vetoes it.
//! - **Position**: to the right of the point, 2 px off, lined up at its
//!   start (reka's fixed `side`, `sideOffset`, `align`), under right to left
//!   too. `gui`'s point placement keeps it on screen.
//! - **Content**: the [`Menu`]'s, modal by default, and unnamed (reka gives
//!   it no `aria-labelledby`: there is no trigger to name it).
//! - **Closing** gives focus back to what held it before it opened (the
//!   engine's focus scope); after a press outside a menu that is not modal,
//!   focus stays where the press put it.
//! - A disabled area opens nothing.
//! - A secondary press on another context menu's area while one is open
//!   closes this one and opens the other at the point, in the same press
//!   (reka's test): a secondary press that closes the top modal layer
//!   reaches what is under it (`gui::interact`), as a platform's context
//!   menu hands it on.

use gui::engine::WorldCell;
use std::time::Duration;

use gui::{Bounds, DismissReason, Div, KeyEvent, NamedKey, PointerButton, Positioning, Side, SideAlign, Signal};

use crate::availability::Availability;
use crate::menu::{Menu, MenuOptions, OpenFocus, OpenModel};

/// The gap between the point and the menu (`ContextMenuContent.vue:58`).
pub const SIDE_OFFSET: f32 = 2.0;

/// A context menu: made once in its root's setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextMenu {
    menu: Menu,
    /// Where the pointer opened it; none: against the area.
    at: Signal<Option<(f32, f32)>>,
    /// The area's box, as last laid out, and whether the coming opening is
    /// the pointer's.
    state: Signal<AreaState>,
}

/// The world's that made the menu: a check writes none of a live one's.
#[derive(Clone, Default)]
struct AreaState {
    area: WorldCell<Option<Bounds>>,
    by_pointer: WorldCell<bool>,
    /// How long a touch or pen is held to open it.
    delay: WorldCell<Option<Duration>>,
}

impl ContextMenu {
    /// A context menu open as `open` says, set up as `options` (reka:
    /// modal; it is unnamed whatever `options` says).
    #[must_use]
    #[track_caller]
    pub fn new(open: impl OpenModel + 'static, options: MenuOptions) -> Self {
        let me = Self {
            menu: Menu::new(open, options.labelled(false)),
            at: Signal::labeled("context menu at", None),
            state: Signal::new(AreaState::default()),
        };
        // An opening that is not the pointer's stands against the area.
        let menu = me.menu;
        gui::watch(
            move || menu.is_open(),
            move |open, _| {
                if *open && me.state.is_alive() && !me.state.with_untracked(|s| s.by_pointer.replace(false)) {
                    me.at.set(None);
                }
            },
        );
        menu.veto(move |reason: gui::DismissReason| me.inside(&reason));
        me
    }

    /// Its menu: the content and items are the engine's parts.
    #[must_use]
    pub fn menu(self) -> Menu {
        self.menu
    }

    /// Open on a touch or pen held `delay` rather than
    /// [`gui::gesture::LONG_PRESS`] (reka's `pressOpenDelay`).
    #[must_use]
    pub fn press_open_delay(self, delay: Duration) -> Self {
        self.state.with_untracked(|s| s.delay.set(Some(delay)));
        self
    }

    /// A secondary press in the area is no press outside.
    fn inside(self, reason: &DismissReason) -> bool {
        let DismissReason::PointerOutside(p) = reason else {
            return false;
        };
        let area = self.state.with_untracked(|s| s.area.get());
        let inside = area.is_some_and(|b| p.x >= b.x && p.x <= b.right() && p.y >= b.y && p.y <= b.bottom());
        if inside && p.button == PointerButton::Secondary {
            self.at.set(Some((p.x, p.y)));
            return true;
        }
        false
    }

    /// Open at `(x, y)`, or move there when open.
    fn open_at(self, x: f32, y: f32) {
        self.at.set(Some((x, y)));
        if !self.menu.is_open_untracked() {
            self.state.with_untracked(|s| s.by_pointer.set(true));
            self.menu.open(OpenFocus::by_modality());
        }
    }

    /// Where the content goes (tracked): at the point the pointer opened it,
    /// else against the area (the deferred content's parent).
    #[must_use]
    pub fn positioning(self) -> Positioning {
        match self.at.get() {
            Some((x, y)) => Positioning::At { x: x + SIDE_OFFSET, y },
            None => Positioning::Anchored {
                side: Side::Right,
                align: SideAlign::Start,
                offset: SIDE_OFFSET,
                match_width: false,
                fit_height: false,
            },
        }
    }

    /// The keys an area takes: the context-menu key and Shift+F10.
    fn key(self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        let menu_key = ev.is(NamedKey::ContextMenu) || (m.shift && ev.is(NamedKey::F10));
        if !menu_key || self.menu.is_open_untracked() {
            return false;
        }
        self.menu.open(OpenFocus::First);
        true
    }

    /// `d` as the area that opens the menu, usable as `availability` says.
    /// Sets the node's secondary click, long press, key and measured
    /// handlers.
    #[must_use]
    pub fn area(self, d: Div, availability: Availability) -> Div {
        if !availability.actionable() {
            return d;
        }
        let state = self.state;
        let delay = state.with_untracked(|s| s.delay.get());
        let d = match delay {
            Some(after) => d.long_press_after(after),
            None => d,
        };
        d.on_measured(move |b| {
            if state.is_alive() {
                state.with_untracked(|s| s.area.set(Some(*b)));
            }
        })
        .on_secondary_click(move |ev| self.open_at(ev.x, ev.y))
        .on_long_press(move |ev| self.open_at(ev.x, ev.y))
        .on_key_down(move |ev| self.key(ev))
    }
}

impl std::fmt::Debug for AreaState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AreaState").finish_non_exhaustive()
    }
}
