//! A row of tabs, each showing its own panel (reka's `Tabs`: `TabsRoot`,
//! `TabsList`, `TabsTrigger`, `TabsContent`, `TabsIndicator`).
//!
//! **Model.** The selected tab's value, the part's `v-model`
//! ([`gui::use_model`]). A tab is selected by pressing it with the primary
//! button (as it goes down, not on release; not with Control, which on a
//! Mac is the secondary click: such a press neither selects nor focuses
//! the tab, reka's `preventDefault`), by Enter or Space, or, when activation is
//! [`Activation::Automatic`], by focus reaching it however it came: the
//! keyboard, code, or a layer giving focus back (`Tabs/TabsTrigger.vue:56-78`). A disabled tab is never
//! selected. The caller's options (orientation, activation, wrap, keeping
//! panels mounted) are read when they are used: reka reads the activation
//! mode only at setup (research §4 item 11).
//!
//! **Keys** ([`crate::Roving`], [`crate::Keymap::TABS`] on the list's axis):
//!
//! | Key | Result |
//! | --- | --- |
//! | Tab into the list | the selected tab; else the one focused last; else the first usable |
//! | Tab from a tab | out of the list, to the panel (a Tab stop) |
//! | Shift+Tab from a tab | out of the list at once |
//! | ArrowRight, ArrowLeft (horizontal, swapped right to left); ArrowDown, ArrowUp (vertical) | next, previous usable tab, wrapping unless [`TabsOptions::wrap`] is [`Wrap::Stop`] |
//! | Home, PageUp; End, PageDown | first; last usable tab |
//! | an arrow across the axis; any navigation key with Control, Alt, Command or Shift | not handled |
//! | Enter, Space | select the focused tab |
//!
//! **To assistive technology.** The list is a tab list with its
//! orientation; each tab a tab, selected or not, controlling its panel
//! while the panel is there (`Tabs/TabsTrigger.vue:30-32`: the engine sends
//! a relation only to a node in the tree); a panel a tab panel named by its
//! tab, always a Tab stop (`Tabs/TabsContent.vue:63-68`).
//!
//! **Panels.** An inactive panel is gone, or hidden and inert when kept
//! mounted (never found by a search: `Tabs/TabsContent.vue:67`). It plays
//! its exit out; one active as it mounts plays no entry (the look's
//! `after_mount` animation).
//!
//! **Indicator** ([`Tabs::indicator`], reka's `TabsIndicator`): where the
//! selected tab lies in its list, measured as layout places them: its
//! offset from the list's padding box along the list's axis, its length
//! along it and its thickness across (`offsetLeft`, `offsetWidth`,
//! `offsetHeight`; `offsetTop` and the others swapped for a vertical
//! list). It follows the selection, the reading direction and every
//! size, since layout measures each tab every pass, and is none until the
//! selected tab has been laid out (`Tabs/TabsIndicator.vue:34-71`). The
//! look draws a bar from it ([`Tabs::indicator_part`]).

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Axis, Bounds, Div, Element, FocusHandle, NamedKey, NodeRef, Signal, UseModel};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::ReadDirection;
use crate::keymap::{Arrows, Keymap, Wrap};
use crate::presence::Presence;
use crate::relation::{Linked, Points};
use crate::roving::{Nav, Roving};

/// When a tab focused from the keyboard is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Activation {
    /// As focus reaches it (reka's default).
    #[default]
    Automatic,
    /// Only by Enter, Space or a press.
    Manual,
}

/// What the caller decides of a tabs, read when it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabsOptions {
    /// The axis its list runs along: horizontal unless set.
    pub orientation: Axis,
    pub activation: Activation,
    /// Whether the arrows go on past the ends (reka's `loop`, on).
    pub wrap: Wrap,
    /// Inactive panels stay mounted, hidden and inert.
    pub keep_mounted: bool,
}

impl Default for TabsOptions {
    fn default() -> Self {
        Self {
            orientation: Axis::Horizontal,
            activation: Activation::Automatic,
            wrap: Wrap::Around,
            keep_mounted: false,
        }
    }
}

impl TabsOptions {
    fn keymap(self) -> Keymap {
        let arrows = match self.orientation {
            Axis::Horizontal => Arrows::Horizontal,
            Axis::Vertical => Arrows::Vertical,
        };
        Keymap::TABS.arrows(arrows).wrap(self.wrap)
    }
}

/// Where the selected tab lies in its list (reka's `TabsIndicator`'s
/// three variables), in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Indicator {
    /// From the list's padding box to the tab, along the list's axis.
    pub position: f32,
    /// The tab's length along the axis.
    pub size: f32,
    /// The tab's thickness across it.
    pub thickness: f32,
}

struct State {
    options: Rc<dyn Fn() -> TabsOptions>,
    /// The tab asked for by its focus arriving or by a press, and the frame
    /// it was in: a press focuses as it selects, and the one act asks once,
    /// whichever of the two runs first.
    /// The world's that made the tabs: a check writes none of live tabs'.
    asked: gui::engine::WorldCell<Option<(gui::focus::FocusId, f32)>>,
}

/// A tabs' state: made once in its root's setup, `Copy`.
pub struct Tabs<P: 'static, T: 'static> {
    value: UseModel<P, T>,
    roving: Roving<T>,
    /// Each mounted tab's border box, as last laid out.
    tabs: Signal<HashMap<T, Bounds>>,
    /// The list's padding box, as last laid out.
    list: Signal<Option<Bounds>>,
    /// Whether the indicator has a place: what its part's presence reads,
    /// not its place, so a move renders nothing.
    placed: gui::Computed<bool>,
    /// Each value's tab and panel nodes, issued as the value is first asked
    /// for, held by its tab and its panel while each is on screen, let go
    /// as the last of them leaves.
    links: gui::Keyed<T, (NodeRef, NodeRef)>,
    state: Signal<State>,
}

impl<P: 'static, T: 'static> Clone for Tabs<P, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static, T: 'static> Copy for Tabs<P, T> {}

impl<P: 'static, T: 'static> PartialEq for Tabs<P, T> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<P: 'static, T: 'static> std::fmt::Debug for Tabs<P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tabs").finish_non_exhaustive()
    }
}

impl<P: 'static, T: Clone + Eq + Hash + 'static> Tabs<P, T> {
    /// Tabs selecting `value` (the part's `v-model`), with the caller's
    /// `options` read when used, navigated in `direction`. Called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, T>,
        options: impl Fn() -> TabsOptions + 'static,
        direction: impl Into<ReadDirection>,
    ) -> Self {
        let options: Rc<dyn Fn() -> TabsOptions> = Rc::new(options);
        let first = gui::untracked(|| options());
        let tabs_bounds: Signal<HashMap<T, Bounds>> = Signal::new(HashMap::new());
        let list: Signal<Option<Bounds>> = Signal::labeled("tabs list bounds", None);
        let placed = gui::Computed::new(move || {
            list.with(Option::is_some) && value.with(|v| tabs_bounds.with(|b| b.contains_key(v)))
        });
        let tabs = Self {
            value,
            roving: Roving::new(first.keymap(), direction),
            tabs: tabs_bounds,
            list,
            placed,
            links: gui::Keyed::new(),
            state: Signal::new(State {
                options: Rc::clone(&options),
                asked: gui::engine::WorldCell::new("the tab asked for (`Tabs`)", None),
            }),
        };
        // The keymap follows the caller's orientation and wrap.
        gui::watch(
            move || options().keymap(),
            move |keymap, _| {
                if tabs.state.is_alive() {
                    tabs.roving.set_keymap(*keymap);
                }
            },
        );
        // Automatic activation: focus reaching a tab selects it, whoever
        // moved it (reka's `@focus`): the keyboard, code, a layer giving it
        // back. A press, which focuses as it selects, selects as it goes
        // down; one that may not select (Control) does not focus either.
        gui::watch(gui::focus::focused, move |now, _| {
            if let Some(f) = *now
                && tabs.state.is_alive()
                && gui::untracked(|| tabs.options()).activation == Activation::Automatic
                && tabs.first_ask(f)
            {
                tabs.select_focused(f);
            }
        });
        // What a tab no longer laid out had is dropped: its box. A pass
        // that laid out the same values as the last does nothing; one that
        // changed them makes a set of them once (linear work), in buffers
        // kept from pass to pass.
        let last: std::cell::Cell<Vec<T>> = std::cell::Cell::default();
        let present: std::cell::Cell<HashSet<T>> = std::cell::Cell::default();
        tabs.roving.items().on_laid_out(move |now| {
            if !tabs.tabs.is_alive() {
                return;
            }
            let mut seen = last.take();
            let same = seen.len() == now.len() && seen.iter().zip(now).all(|(a, b)| *a == b.value);
            if !same {
                seen.clear();
                seen.extend(now.iter().map(|i| i.value.clone()));
                let mut set = present.take();
                set.clear();
                set.extend(seen.iter().cloned());
                if tabs.tabs.with_untracked(|b| b.keys().any(|v| !set.contains(v))) {
                    tabs.tabs.update(|b| b.retain(|v, _| set.contains(v)));
                }
                present.set(set);
            }
            last.set(seen);
        });
        tabs
    }

    /// The caller's options now (tracked through what its closure reads).
    #[must_use]
    pub fn options(self) -> TabsOptions {
        let f = self.state.with_untracked(|s| Rc::clone(&s.options));
        f()
    }

    /// The selected value (tracked).
    #[must_use]
    pub fn value(self) -> T {
        self.value.get()
    }

    /// Whether `value` is selected (tracked): what a tab's and a panel's
    /// looks read (`data-state`).
    #[must_use]
    pub fn is_selected(self, value: &T) -> bool {
        self.value.with(|v| v == value)
    }

    /// The tabs, one Tab stop among them.
    #[must_use]
    pub fn roving(self) -> Roving<T> {
        self.roving
    }

    /// Select `value`: the user chose it, or the caller did.
    pub fn select(self, value: T) {
        self.value.set(value);
    }

    /// Whether the tab of `focus` was not asked for yet in this frame (by
    /// the other of a press and its focus); it is from now.
    fn first_ask(self, focus: gui::focus::FocusId) -> bool {
        let now = (focus, gui::clock::frame_time());
        self.state.with_untracked(|s| s.asked.replace(Some(now))) != Some(now)
    }

    fn select_focused(self, focus: gui::focus::FocusId) {
        let found = self.roving.items().with(|items| {
            items
                .iter()
                .find(|i| i.focus.id() == focus && i.availability.actionable())
                .map(|i| i.value.clone())
        });
        if let Some(value) = found {
            self.select(value);
        }
    }

    /// How many values the tabs keep anything of: the mounted tabs' boxes,
    /// and the nodes issued to the values (let go as the last tab or panel
    /// of a value leaves).
    #[must_use]
    pub fn kept_tabs(self) -> usize {
        self.tabs.with_untracked(HashMap::len).max(self.links.len())
    }

    /// The nodes of `value`'s tab and panel, held while the calling render's
    /// output may be on screen.
    fn link(self, value: &T) -> Linked {
        let (trigger, content) = self.links.hold(value, || (NodeRef::new(), NodeRef::new()));
        Linked {
            trigger,
            content,
            points: Points::Controls,
        }
    }

    /// `d` as the list: a tab list with its orientation, measured for the
    /// indicator. Sets `on_measured`.
    #[must_use]
    pub fn list(self, mut d: Div) -> Div {
        let border = d.style_mut().border_widths;
        d.role(gui::Role::TabList)
            .aria_orientation(self.options().orientation)
            .on_measured(move |b| {
                let padding_box = Bounds {
                    x: b.x + border.left,
                    y: b.y + border.top,
                    width: (b.width - border.left - border.right).max(0.0),
                    height: (b.height - border.top - border.bottom).max(0.0),
                };
                if self.list.is_alive() && self.list.get_untracked() != Some(padding_box) {
                    self.list.set(Some(padding_box));
                }
            })
    }

    /// Where the selected tab lies in the list (tracked): none until both
    /// have been laid out. What the look's indicator reads.
    #[must_use]
    pub fn indicator(self) -> Option<Indicator> {
        let list = self.list.get()?;
        let tab = self.value.with(|v| self.tabs.with(|b| b.get(v).copied()))?;
        Some(match self.options().orientation {
            Axis::Horizontal => Indicator {
                position: tab.x - list.x,
                size: tab.width,
                thickness: tab.height,
            },
            Axis::Vertical => Indicator {
                position: tab.y - list.y,
                size: tab.height,
                thickness: tab.width,
            },
        })
    }

    /// `d`, the look's bar, as the indicator: placed absolutely in the list
    /// (which the look positions) at the selected tab, its offset and length
    /// bound to the node, so a new selection or size moves it without a
    /// render; none until the tab has been laid out. The look sets its
    /// thickness, or reads [`Indicator::thickness`].
    #[must_use]
    pub fn indicator_part(self, d: Div) -> Option<Div> {
        if !self.placed.get() {
            return None;
        }
        let at = move || self.indicator().unwrap_or_default();
        let d = d.position(Position::Absolute);
        Some(match gui::untracked(|| self.options()).orientation {
            Axis::Horizontal => d
                .left(move || Length::Px(at().position))
                .width(move || Length::Px(at().size)),
            Axis::Vertical => d
                .top(move || Length::Px(at().position))
                .height(move || Length::Px(at().size)),
        })
    }

    /// `d`, the look's tab, as the tab of `value`, usable as
    /// `availability` says, focused through `focus` (the trigger's own,
    /// made in its setup). Sets `on_mouse_down`, `focus_on_press`,
    /// `on_measured` and `on_key_down`.
    #[must_use]
    pub fn trigger(self, d: Div, value: T, availability: Availability, focus: FocusHandle) -> Div {
        let selected = self.is_selected(&value);
        let d = d.role(gui::Role::Tab).aria_selected(selected);
        let d = self.link(&value).trigger(d, true);
        let item = Item::new(value.clone(), focus)
            .active(selected)
            .availability(availability);
        let (pressed, measured) = (value.clone(), value.clone());
        // Its own keys below, the group's included (handlers add up).
        self.roving
            .member(d, item)
            .on_mouse_down(move |ev| {
                if availability.actionable() && !ev.modifiers.control && self.first_ask(focus.id()) {
                    self.select(pressed.clone());
                }
            })
            // A Control press is a Mac's secondary click: it neither selects
            // nor focuses (reka's `preventDefault`).
            .focus_on_press(|m| !m.control)
            .on_measured(move |b| {
                if self.tabs.is_alive() && self.tabs.with_untracked(|t| t.get(&measured) != Some(b)) {
                    self.tabs.update(|t| {
                        t.insert(measured.clone(), *b);
                    });
                }
            })
            .on_key_down(move |ev| match self.roving.key(focus, ev) {
                Nav::Ignored => {
                    let activates = ev.is(NamedKey::Enter) || ev.is(NamedKey::Space);
                    if activates && availability.actionable() {
                        self.select(value.clone());
                    }
                    activates
                }
                nav => nav.consumed(),
            })
    }

    /// Where the panel of `value` is now (tracked).
    #[must_use]
    pub fn presence(self, value: &T) -> Presence {
        Presence::of(self.is_selected(value), self.options().keep_mounted)
    }

    /// `d`, the look's panel, as the panel of `value`: a tab panel named
    /// by its tab and a Tab stop, there while its tab is selected, kept
    /// hidden and inert otherwise when panels are kept mounted, else gone.
    #[must_use]
    pub fn panel(self, d: Div, value: &T) -> Element {
        let d = d.role(gui::Role::TabPanel).focusable();
        self.presence(value).render(self.link(value).content(d, true))
    }
}
