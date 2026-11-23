//! A row (or column) of controls that is one Tab stop (reka's `Toolbar`;
//! research §8, "Toolbar"), on [`Roving`].
//!
//! - **One Tab stop.** Tab enters at the item focused last, else the first
//!   enabled one, and leaves at once; Shift+Tab leaves too. A toolbar whose
//!   items are all disabled is no stop. A disabled item takes no focus, the
//!   pointer's included.
//! - **Keys** ([`Keymap::TOOLBAR`]): ArrowRight and ArrowLeft along a
//!   horizontal toolbar (swapped right to left), ArrowDown and ArrowUp
//!   along a vertical one; the other axis does nothing. Home and PageUp,
//!   End and PageDown reach the ends. Past an end it stops, unless the
//!   caller wraps it (reka's `loop`, which is off: an absent Boolean prop is
//!   `false` in Vue). No arrow moves with Control, Alt, Command or Shift.
//! - **Items**: a button and a toggle item press as buttons do (Enter,
//!   Space); a link follows on Enter and on Space too (`ToolbarLink`). A
//!   toggle group ([`Toolbar::toggle_group`], [`Toolbar::toggle_item`],
//!   reka's `ToolbarToggleGroup`, `ToolbarToggleItem`) is a `group` whose
//!   items are the toolbar's own: one Tab stop for the whole toolbar, the
//!   toolbar's arrows, each item a toggle pressed while the group holds its
//!   value (reka's group with `rovingFocus` off inside the toolbar's).
//! - **Controls with keys of their own** (a dropdown menu's trigger, a
//!   select's) keep them. The arrows are the root's: an item sets no key
//!   handler, and the root takes what the focused item's own handler left
//!   as it bubbles up (reka binds the group's listener after the item's,
//!   and an item's handler that takes a key stops it). So a trigger made an
//!   item, in either order (`toolbar.item(dd.trigger(d, ..), ..)` or
//!   `dd.trigger(toolbar.item(d, ..), ..)`), opens its menu on ArrowDown, in
//!   a vertical toolbar too, while the arrows it leaves move along the
//!   toolbar.
//! - **AT.** The toolbar is a `toolbar` with its orientation. A separator
//!   in it turns across the toolbar (vertical in a horizontal one), where
//!   reka's `ToolbarSeparator` takes the toolbar's own orientation: see
//!   [`Toolbar::separator`] for why.

use std::rc::Rc;

use gui::{Axis, Direction, Div, FocusHandle, KeyEvent, Role};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::use_direction;
use crate::keymap::{Arrows, Keymap, Wrap};
use crate::roving::{Nav, Roving};
use crate::toggle;
use crate::toggle_group::{Pressing, ToggleGroup};

/// A toolbar: made once in its root's setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Toolbar {
    roving: Roving<FocusHandle>,
    orientation: Axis,
}

impl Toolbar {
    /// A toolbar laid out along `orientation`, wrapping as `wrap` says
    /// (reka: horizontal, no wrap), in `direction` (none: the nearest
    /// provided); called in the root's setup.
    #[must_use]
    pub fn new(orientation: Axis, wrap: Wrap, direction: Option<Direction>) -> Self {
        let arrows = match orientation {
            Axis::Horizontal => Arrows::Horizontal,
            Axis::Vertical => Arrows::Vertical,
        };
        Self {
            roving: Roving::new(Keymap::TOOLBAR.arrows(arrows).wrap(wrap), use_direction(direction)),
            orientation,
        }
    }

    /// Its orientation, for the look.
    #[must_use]
    pub fn orientation(self) -> Axis {
        self.orientation
    }

    /// `d` as the toolbar: a `toolbar` with its orientation, taking the
    /// arrows its focused item leaves (it sets the node's key handler).
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(Role::Toolbar)
            .aria_orientation(self.orientation)
            .on_key_down(move |ev| self.key(ev).consumed())
    }

    /// `d` as an item focused through `focus`, usable as `availability`
    /// says: it joins the group, whose arrows the root takes. It sets no
    /// key handler, so a control's own (a menu's trigger) composes with it
    /// in either order.
    #[must_use]
    pub fn item(self, d: Div, focus: FocusHandle, availability: Availability) -> Div {
        self.roving
            .member(d, Item::new(focus, focus).availability(availability))
    }

    /// `d` as a link of the toolbar: an item that Space follows too.
    #[must_use]
    pub fn link(self, d: Div, focus: FocusHandle, availability: Availability, follow: impl Fn() + 'static) -> Div {
        let follow = Rc::new(follow);
        let on_key = Rc::clone(&follow);
        let usable = availability.actionable();
        self.item(d, focus, availability)
            .role(Role::Link)
            .on_click(move |_| {
                if usable {
                    follow();
                }
            })
            .on_key_down(move |ev| {
                if usable && crate::keymap::activates(ev) {
                    on_key();
                    return true;
                }
                false
            })
    }

    /// `d` as a toggle group of the toolbar: a `group`; its items are
    /// [`Self::toggle_item`]s.
    #[must_use]
    pub fn toggle_group<P: 'static, V: Pressing<T>, T: Clone + PartialEq + 'static>(
        self,
        d: Div,
        group: ToggleGroup<P, V, T>,
    ) -> Div {
        group.root(d)
    }

    /// `d` as the toggle of `value` in `group`, an item of the toolbar
    /// focused through `focus`: pressed while the group holds `value`, a
    /// press flips it in the group; usable as `availability` says. Called
    /// in render: it reads whether it is pressed (tracked).
    #[must_use]
    pub fn toggle_item<P: 'static, V: Pressing<T>, T: Clone + PartialEq + 'static>(
        self,
        d: Div,
        group: ToggleGroup<P, V, T>,
        value: T,
        focus: FocusHandle,
        availability: Availability,
    ) -> Div {
        let pressed = group.pressed(&value);
        let d = toggle::root(d, pressed, availability, move |_| group.flip(&value));
        self.item(d, focus, availability)
    }

    /// What `ev` does at the toolbar: an arrow moves from the focused item,
    /// nothing while focus is on none of its items.
    pub fn key(self, ev: &KeyEvent) -> Nav<FocusHandle> {
        self.roving.focused_key(ev)
    }

    /// `d` as a separator of the toolbar.
    ///
    /// reka gives it the toolbar's own orientation, so a horizontal
    /// toolbar's separator is read as horizontal although it stands upright
    /// between the items. ARIA's separator orientation is the line's own
    /// (WAI-ARIA 1.2, `separator`: a vertical one divides content placed
    /// side by side), and the toolbar pattern's separators turn across the
    /// toolbar (APG's toolbar example, Chromium's own toolbars). Here it is
    /// perpendicular to the toolbar: vertical in a horizontal toolbar
    /// (research §4 item 12, ADR-0025 "Group 4").
    #[must_use]
    pub fn separator(self, d: Div) -> Div {
        let across = match self.orientation {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
        crate::separator::Separator::new(across, false).apply(d)
    }
}
