//! Stacked sections, each a heading that opens its own content (reka's
//! `Accordion`: `AccordionRoot`, `AccordionItem`, `AccordionHeader`,
//! `AccordionTrigger`, `AccordionContent`).
//!
//! **Model.** Which sections are open, typed by how many may be (research
//! §3.2): one, an `Option<T>` ([`Accordion::single`]), or any number, a
//! `Vec<T>` in the order they were opened ([`Accordion::multiple`];
//! `shared/useSingleOrMultipleValue.ts:53-67`). reka's root takes one value
//! that is sometimes a string and sometimes an array and guesses which from
//! it (`:18-33`); here the type says it. Pressing a trigger:
//!
//! | Kind | Section closed | Section open |
//! | --- | --- | --- |
//! | one | it opens, the open one closes | it closes when the accordion is collapsible, else nothing happens and the caller is not told |
//! | many | it is added last | it is taken out |
//!
//! A disabled section, or a disabled accordion, writes nothing
//! (`Accordion/AccordionTrigger.vue:20-26`). The caller's options
//! (collapsible, orientation, availability, keeping content mounted) are
//! read when they are used, so changing one later counts at once: reka reads
//! `collapsible` and `orientation` only at setup (research §4 item 11).
//!
//! **Keys** (focus on a trigger; [`crate::Keymap::ACCORDION`]):
//!
//! | Key | Result |
//! | --- | --- |
//! | Enter, Space | press the trigger (a button) |
//! | Tab, Shift+Tab | each usable trigger is a Tab stop; open content lies between, in tree order |
//! | ArrowDown, ArrowUp (vertical); ArrowRight, ArrowLeft (horizontal, swapped right to left) | next, previous usable trigger, wrapping |
//! | Home, End, PageUp, PageDown | first, last usable trigger |
//! | an arrow across the axis | not handled: the page scrolls |
//!
//! The arrows only move focus; they open nothing
//! (`Accordion/AccordionItem.vue:101-110`). Two of reka's quirks are not
//! kept (research §4 item 13, ADR-0025 group 8): its Home and End reach a
//! disabled trigger (in Chrome focus then stays, a `disabled` button
//! taking none) where its arrows skip it; and its arrows walk every
//! collection item under the root, so a nested accordion's triggers, or
//! tabs inside open content, join one flat ring. Here the arrows walk only
//! this accordion's own triggers ([`Accordion::triggers`]), disabled ones
//! skipped by every key.
//!
//! **To assistive technology.** Each header is a heading of level 3
//! (`Accordion/AccordionHeader.vue:13-15`); its trigger a button,
//! `aria-expanded`, controlling its content while that is there; a disabled
//! trigger is disabled. A section that is open and may not close is not
//! read as disabled and keeps focus (`:36`). Content is a region named by
//! its trigger (`Accordion/AccordionContent.vue:23-27`).

use std::rc::Rc;

use gui::{Axis, Div, Element, FocusHandle, KeyEvent, Signal, UseModel};

use crate::availability::Availability;
use crate::collection::{Collection, Item};
use crate::direction::ReadDirection;
use crate::keymap::{Arrows, Keymap};
use crate::presence::Presence;
use crate::relation::{Linked, Points};

/// How the open sections are held: one or many.
pub trait OpenSections<T>: 'static {
    /// Whether the section `value` is open (tracked).
    fn is_open(&self, value: &T) -> bool;
    /// The user pressed the trigger of `value`; one open section closes
    /// only when `collapsible`.
    fn toggle(&self, value: &T, collapsible: bool);
}

/// One section open at most: its value, or none.
impl<P: 'static, T: Clone + PartialEq + 'static> OpenSections<T> for UseModel<P, Option<T>> {
    fn is_open(&self, value: &T) -> bool {
        self.with(|open| open.as_ref() == Some(value))
    }

    fn toggle(&self, value: &T, collapsible: bool) {
        let was_open = gui::untracked(|| self.with(|open| open.as_ref() == Some(value)));
        match (was_open, collapsible) {
            // reka's trigger returns before it writes: nothing is told.
            (true, false) => {}
            (true, true) => self.set(None),
            (false, _) => self.set(Some(value.clone())),
        }
    }
}

/// Any number open: their values, in the order they were opened.
impl<P: 'static, T: Clone + PartialEq + 'static> OpenSections<T> for UseModel<P, Vec<T>> {
    fn is_open(&self, value: &T) -> bool {
        self.with(|open| open.contains(value))
    }

    fn toggle(&self, value: &T, _collapsible: bool) {
        self.update(|open| {
            if let Some(at) = open.iter().position(|v| v == value) {
                open.remove(at);
            } else {
                open.push(value.clone());
            }
        });
    }
}

/// What the caller decides of an accordion, read when it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccordionOptions {
    /// Every section cannot be used.
    pub availability: Availability,
    /// The axis its triggers run along: vertical unless set.
    pub orientation: Axis,
    /// With one section open at most: whether it may close, leaving none.
    pub collapsible: bool,
    /// Closed sections' content stays mounted, hidden and inert.
    pub keep_mounted: bool,
}

impl Default for AccordionOptions {
    fn default() -> Self {
        Self {
            availability: Availability::Enabled,
            orientation: Axis::Vertical,
            collapsible: false,
            keep_mounted: false,
        }
    }
}

struct State<T: 'static> {
    open: Rc<dyn OpenSections<T>>,
    options: Rc<dyn Fn() -> AccordionOptions>,
    /// This accordion's triggers, in tree order as last laid out.
    triggers: Collection<()>,
    direction: ReadDirection,
}

/// An accordion's state: made once in its root's setup, `Copy`.
pub struct Accordion<T: 'static> {
    state: Signal<State<T>>,
}

impl<T: 'static> Clone for Accordion<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Accordion<T> {}

impl<T: 'static> PartialEq for Accordion<T> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<T: 'static> std::fmt::Debug for Accordion<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Accordion").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static> Accordion<T> {
    /// One section open at most, the one `open` holds (the part's
    /// `v-model`), with the caller's `options` read when used; navigated in
    /// `direction`. Called in setup.
    #[must_use]
    pub fn single<P: 'static>(
        open: UseModel<P, Option<T>>,
        options: impl Fn() -> AccordionOptions + 'static,
        direction: impl Into<ReadDirection>,
    ) -> Self {
        Self::new(Rc::new(open), Rc::new(options), direction.into())
    }

    /// Any number of sections open, those `open` holds.
    #[must_use]
    pub fn multiple<P: 'static>(
        open: UseModel<P, Vec<T>>,
        options: impl Fn() -> AccordionOptions + 'static,
        direction: impl Into<ReadDirection>,
    ) -> Self {
        Self::new(Rc::new(open), Rc::new(options), direction.into())
    }

    fn new(open: Rc<dyn OpenSections<T>>, options: Rc<dyn Fn() -> AccordionOptions>, direction: ReadDirection) -> Self {
        Self {
            state: Signal::new(State {
                open,
                options,
                triggers: Collection::new(),
                direction,
            }),
        }
    }

    /// The caller's options now (tracked through what its closure reads).
    #[must_use]
    pub fn options(self) -> AccordionOptions {
        let f = self.state.with_untracked(|s| Rc::clone(&s.options));
        f()
    }

    /// The triggers, in tree order as last laid out.
    #[must_use]
    pub fn triggers(self) -> Collection<()> {
        self.state.with_untracked(|s| s.triggers.clone())
    }

    /// Whether section `value` is open (tracked).
    #[must_use]
    pub fn is_open(self, value: &T) -> bool {
        let open = self.state.with_untracked(|s| Rc::clone(&s.open));
        open.is_open(value)
    }

    /// The user pressed the trigger of a section of `value` whose own
    /// availability is `own`.
    pub fn toggle(self, value: &T, own: Availability) {
        let options = gui::untracked(|| self.options());
        if !options.availability.actionable() || !own.actionable() {
            return;
        }
        let open = self.state.with_untracked(|s| Rc::clone(&s.open));
        open.toggle(value, options.collapsible);
    }

    /// The section `item` (an item's props): made once in the item's setup.
    #[must_use]
    pub fn section(self, item: SectionItem<T>) -> Section<T> {
        Section {
            accordion: self,
            item: Signal::new(item),
            linked: Linked::new(Points::Controls),
            focus: FocusHandle::new(),
        }
    }

    /// Move focus from the trigger `from` as `ev` asks; whether the key was
    /// one of the accordion's.
    fn navigate(self, from: FocusHandle, ev: &KeyEvent) -> bool {
        let options = gui::untracked(|| self.options());
        let arrows = match options.orientation {
            Axis::Horizontal => Arrows::Horizontal,
            Axis::Vertical => Arrows::Vertical,
        };
        let keymap = Keymap::ACCORDION.arrows(arrows);
        let (triggers, direction) = self.state.with_untracked(|s| (s.triggers.clone(), s.direction));
        let Some(intent) = keymap.intent(ev, direction.get()) else {
            return false;
        };
        let at = triggers.index_of(from.id());
        let to = triggers.with(|list| keymap.step(list, at, intent).and_then(|i| list.get(i)).map(|t| t.focus));
        if let Some(to) = to {
            gui::focus::set_focus(Some(to.id()), true);
        }
        true
    }
}

/// What a section is given (reka's `AccordionItem` props).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionItem<T> {
    /// The value it opens as.
    pub value: T,
    /// Its own availability (reka's `disabled` on an item).
    pub availability: Availability,
    /// Its own choice of keeping its content mounted while closed, over the
    /// accordion's (`Accordion/AccordionItem.vue:46-51`, `:123`).
    pub keep_mounted: Option<bool>,
}

impl<T> SectionItem<T> {
    /// An enabled section of `value` that keeps content as its accordion does.
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self {
            value,
            availability: Availability::Enabled,
            keep_mounted: None,
        }
    }
}

/// One section of an [`Accordion`]: its props, its trigger's focus and the
/// trigger ↔ content relation. Made once in the item's setup, `Copy`.
pub struct Section<T: 'static> {
    accordion: Accordion<T>,
    item: Signal<SectionItem<T>>,
    linked: Linked,
    focus: FocusHandle,
}

impl<T: 'static> Clone for Section<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Section<T> {}

impl<T: 'static> PartialEq for Section<T> {
    fn eq(&self, other: &Self) -> bool {
        self.item == other.item
    }
}

impl<T: 'static> std::fmt::Debug for Section<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Section").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static> Section<T> {
    /// Give it `item` from now on (the item's props changed); an equal one
    /// changes nothing.
    pub fn set(self, item: SectionItem<T>) {
        if self.item.with_untracked(|i| *i != item) {
            self.item.set(item);
        }
    }

    /// Whether it is open (tracked): what its trigger, header and content
    /// looks read.
    #[must_use]
    pub fn is_open(self) -> bool {
        let value = self.item.with(|i| i.value.clone());
        self.accordion.is_open(&value)
    }

    /// Whether its trigger can be used (tracked): not when it or the whole
    /// accordion cannot (`Accordion/AccordionItem.vue:69-71`).
    #[must_use]
    pub fn availability(self) -> Availability {
        let own = self.item.with(|i| i.availability);
        let all = self.accordion.options().availability;
        match (all, own) {
            (Availability::Disabled, _) | (_, Availability::Disabled) => Availability::Disabled,
            (Availability::Unavailable, _) | (_, Availability::Unavailable) => Availability::Unavailable,
            _ => Availability::Enabled,
        }
    }

    /// The accordion's orientation (tracked), which the look's
    /// `data-orientation` reads.
    #[must_use]
    pub fn orientation(self) -> Axis {
        self.accordion.options().orientation
    }

    /// `d` as the header: a heading of level 3.
    #[must_use]
    pub fn header(self, d: Div) -> Div {
        d.role(gui::Role::Heading).aria_level(3)
    }

    /// `d`, the look's button, as the trigger: a button, expanded while
    /// open, pointing at the content; one of the accordion's triggers for
    /// its arrow keys; pressed, it toggles the section. Sets `on_click` and
    /// `on_key_down`.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let open = self.is_open();
        let availability = self.availability();
        let d = d.role(gui::Role::Button).aria_expanded(open);
        let d = self.linked.trigger(d, open);
        let accordion = self.accordion;
        let focus = self.focus;
        accordion
            .triggers()
            .item(d, Item::new((), focus).availability(availability))
            .on_click(move |_| {
                let (value, own) = self.item.with_untracked(|i| (i.value.clone(), i.availability));
                accordion.toggle(&value, own);
            })
            .on_key_down(move |ev| {
                gui::untracked(gui::focus::focused) == Some(focus.id()) && accordion.navigate(focus, ev)
            })
    }

    /// Where its content is now (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        let keep = self
            .item
            .with(|i| i.keep_mounted)
            .unwrap_or_else(|| self.accordion.options().keep_mounted);
        Presence::of(self.is_open(), keep)
    }

    /// `d`, the look's content box, as the content: a region named by the
    /// trigger, there while open, kept hidden and inert while closed when
    /// the accordion keeps content mounted, gone otherwise.
    #[must_use]
    pub fn content(self, d: Div) -> Element {
        self.presence()
            .render(self.linked.content(d.role(gui::Role::Region), true))
    }
}
