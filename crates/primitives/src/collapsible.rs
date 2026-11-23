//! Content a trigger shows and hides (reka's `Collapsible`: `CollapsibleRoot`,
//! `CollapsibleTrigger`, `CollapsibleContent`).
//!
//! - **Model.** Whether it is open, a `bool` the part's `v-model:open`
//!   holds ([`gui::use_model`]); closed unless the caller says otherwise.
//!   The trigger toggles it ([`Collapsible::toggle`]), unless the
//!   collapsible cannot be used; the caller's own writes
//!   ([`Collapsible::set_open`]) go through whatever its availability, as
//!   reka's `CollapsibleRoot` lets a disabled one be opened by its owner
//!   (`Collapsible/CollapsibleRoot.vue:67-72`).
//! - **Trigger.** A button to assistive technology, `aria-expanded` while
//!   open, pointing at the content (`aria-controls`) while it is there
//!   ([`crate::relation::Linked`]); Enter and Space press it as they press
//!   any button. A disabled one is no Tab stop and takes no press
//!   ([`crate::Availability`]).
//! - **Content.** Gone from the tree while closed, unless kept mounted
//!   (reka's `unmountOnHide: false`), when it stays hidden and inert
//!   ([`crate::presence::Presence::Kept`]). Closing, it plays its exit
//!   animation out (the engine's, ADR-0007), read as closed at once; one
//!   open as it mounts plays no entry, and its height animates from the
//!   size layout gives it, which the look's animation reads
//!   (`gui::Animation::accordion_down().after_mount()`): reka measures the
//!   node with its animations blocked to learn the same size, and keeps a
//!   flag until the first frame for the same "no animation at mount"
//!   (`Collapsible/CollapsibleContent.vue:53-96`).
//!
//! reka also lets find-in-page reveal kept content (`hidden="until-found"`
//! and `beforematch`, which opens it through the model). `gui` has no
//! find-in-page, so this is not built: kept content is hidden and inert,
//! as from everything else (ADR-0025, group 8, item 11).

use std::rc::Rc;

use gui::{Div, Element, Signal, UseModel};

use crate::availability::Availability;
use crate::presence::Presence;
use crate::relation::{Linked, Points};

/// A collapsible's state: made once in its root's setup, `Copy`.
pub struct Collapsible<P: 'static> {
    open: UseModel<P, bool>,
    availability: Signal<Rc<dyn Fn() -> Availability>>,
    keep_mounted: Signal<Rc<dyn Fn() -> bool>>,
    linked: Linked,
}

impl<P: 'static> Clone for Collapsible<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Collapsible<P> {}

impl<P: 'static> PartialEq for Collapsible<P> {
    fn eq(&self, other: &Self) -> bool {
        self.linked == other.linked
    }
}

impl<P: 'static> std::fmt::Debug for Collapsible<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collapsible").finish_non_exhaustive()
    }
}

impl<P: 'static> Collapsible<P> {
    /// A collapsible open as `open` says (the part's `v-model:open`), usable
    /// as `availability` says and kept mounted while closed when
    /// `keep_mounted` says so (both read when asked, so a caller's change
    /// counts at once); called in setup.
    #[must_use]
    pub fn new(
        open: UseModel<P, bool>,
        availability: impl Fn() -> Availability + 'static,
        keep_mounted: impl Fn() -> bool + 'static,
    ) -> Self {
        let availability: Rc<dyn Fn() -> Availability> = Rc::new(availability);
        let keep_mounted: Rc<dyn Fn() -> bool> = Rc::new(keep_mounted);
        Self {
            open,
            availability: Signal::new(availability),
            keep_mounted: Signal::new(keep_mounted),
            linked: Linked::new(Points::Controls),
        }
    }

    /// Whether it is open (tracked): what the trigger's and the content's
    /// looks read (`data-state`).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    /// Whether it can be used (tracked through what the caller's closure
    /// reads): what the trigger's look dims by.
    #[must_use]
    pub fn availability(self) -> Availability {
        let f = self.availability.get_untracked();
        f()
    }

    /// Where its content is now (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        let keep = self.keep_mounted.get_untracked();
        Presence::of(self.is_open(), keep())
    }

    /// The user pressed the trigger: open it when closed, close it when
    /// open, unless it cannot be used.
    pub fn toggle(self) {
        if gui::untracked(|| self.availability()).actionable() {
            self.open.update(|o| *o = !*o);
        }
    }

    /// The caller's write: open or close it whatever its availability.
    pub fn set_open(self, open: bool) {
        self.open.set(open);
    }

    /// `d`, the look's button, as the trigger: a Tab stop (reka's
    /// `<button>`) that toggles the content, told expanded while open and
    /// pointing at the content while it is there. Sets `on_click`.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let open = self.is_open();
        let d = d.role(gui::Role::Button).focusable().aria_expanded(open);
        let d = self.availability().apply(d);
        self.linked.trigger(d, open).on_click(move |_| self.toggle())
    }

    /// `d`, the look's content box, as the content: there while open, kept
    /// hidden and inert while closed when kept mounted, gone otherwise.
    #[must_use]
    pub fn content(self, d: Div) -> Element {
        self.presence().render(self.linked.content(d, false))
    }

    /// The trigger and content's relation, for a part that names one by the
    /// other (an accordion's region by its heading).
    #[must_use]
    pub fn linked(self) -> Linked {
        self.linked
    }
}
