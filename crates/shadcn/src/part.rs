//! Compound parts as component instances (ADR-0013).
//!
//! A part (`tabs`, `tabs_trigger`, `accordion_content`, …) is a box its
//! caller styles and fills as any `Div`, given to a component instance as its
//! slot. The instance keeps the part's state (a root makes it in `setup` and
//! [`gui::provide`]s it; the parts below it [`gui::use_context`] it), and its
//! render adds what that state says to the box: a selected trigger's colours,
//! a closed panel's absence. A write to the state renders only the parts that
//! read it.
//!
//! A `v-model` prop is a [`gui::Bind`] in the part's props, read in setup
//! through [`gui::use_model`]: the caller's model, a value it keeps, or the
//! part's own from a default, as reka's `useVModel` does.

use std::panic::Location;

use gui::{
    ComponentElement, Div, Element, ElementKey, Fluent, IntoElement, Signal, Slot, Style, Styled, TextField, UseModel,
    watch,
};
use primitives::context::use_root_at;

/// Called in setup: `field` and the text `value` (a part's `v-model`) stay
/// one. An edit writes the text up; a text written from outside (a form's
/// reset, a sibling sharing the model) is what the field then shows.
pub(crate) fn bind_field<P: 'static>(field: TextField, value: UseModel<P, String>) {
    watch(move || field.text(), move |text, _| value.set(text.clone()));
    watch(
        move || value.get(),
        move |text, _| {
            if field.edit.with_untracked(|e| e.text() != *text) {
                field.set_text(text.clone());
            }
        },
    );
}

/// What a part's instance is given: the part's own props and its slot.
#[derive(Clone)]
pub struct PartProps<P> {
    pub props: P,
    /// The box the caller styled and filled (never equal: its parent's
    /// render makes it anew, so the part renders again with its parent).
    pub slot: Slot<Div>,
    /// Where the caller's code made the part (a report of a missing root
    /// names it).
    pub at: &'static Location<'static>,
}

/// The state of the root `root` that the part `part`, whose props are
/// `props`, sits in; called in its setup. Outside the root it is the
/// reported error of [`use_root`], naming where the caller made the part.
#[must_use]
pub(crate) fn part_root<P: 'static, T: Clone + 'static>(
    props: Signal<PartProps<P>>,
    part: &'static str,
    root: &'static str,
) -> Option<T> {
    use_root_at(part, root, part_at(props))
}

/// Where the caller's code made the part whose props are `props`.
#[must_use]
pub(crate) fn part_at<P: 'static>(props: Signal<PartProps<P>>) -> &'static Location<'static> {
    props.with_untracked(|p| p.at)
}

impl<P> PartialEq for PartProps<P> {
    fn eq(&self, _: &Self) -> bool {
        // The slot never compares equal.
        false
    }
}

/// A part of a compound component: style and fill it as a `Div`; it becomes
/// its instance when it goes into a tree.
pub struct Part<P: 'static> {
    props: P,
    root: Div,
    key: Option<ElementKey>,
    mount: fn(PartProps<P>) -> ComponentElement,
    at: &'static Location<'static>,
}

impl<P: 'static> Part<P> {
    /// A part whose box starts as `root`, its instance made by `mount`
    /// (a call of [`gui::component`]).
    ///
    /// Its place for a report is where this is called from; a public
    /// constructor that can report is `#[track_caller]`, so that place is
    /// the caller's own code (or [`Part::made_at`] sets it).
    #[must_use]
    #[track_caller]
    pub fn new(props: P, root: Div, mount: fn(PartProps<P>) -> ComponentElement) -> Self {
        Self {
            props,
            root,
            key: None,
            mount,
            at: Location::caller(),
        }
    }

    /// Say where the caller's code made the part, when `new` runs later
    /// than the caller's call (in a closure the constructor kept).
    #[must_use]
    pub fn made_at(mut self, at: &'static Location<'static>) -> Self {
        self.at = at;
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.root = self.root.child(child);
        self
    }

    #[must_use]
    pub fn children<E: IntoElement>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.root = self.root.children(children);
        self
    }

    /// Change the box as a `Div` (a handler, a name, a role).
    #[must_use]
    pub fn map_root(mut self, f: impl FnOnce(Div) -> Div) -> Self {
        self.root = f(self.root);
        self
    }

    /// The name the box shows in the debug tools and the verdict.
    #[must_use]
    pub fn named(self, name: &'static str) -> Self {
        self.map_root(|d| d.named(name))
    }

    /// Called on a click on the box, before the part's own click (handlers
    /// add up: each one registered runs, in the order registered).
    #[must_use]
    pub fn on_click(self, f: impl Fn(&gui::ClickEvent) + 'static) -> Self {
        self.map_root(|d| d.on_click(f))
    }

    /// Its identity among its siblings (two parts of one kind side by side
    /// keep their state when one before them comes or goes).
    #[must_use]
    pub fn key(mut self, key: impl Into<ElementKey>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The part's props, to set from a builder method.
    pub fn props_mut(&mut self) -> &mut P {
        &mut self.props
    }

    /// Set a prop: `f` edits the props.
    #[must_use]
    pub fn with_props(mut self, f: impl FnOnce(&mut P)) -> Self {
        f(&mut self.props);
        self
    }
}

impl<P: 'static> Fluent for Part<P> {}

impl<P: 'static> Styled for Part<P> {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.root.style_and_bindings()
    }

    fn node_interaction(&self) -> Option<gui::Interaction> {
        self.root.node_interaction()
    }
}

impl<P: 'static> IntoElement for Part<P> {
    fn into_element(self) -> Element {
        let el = (self.mount)(PartProps {
            props: self.props,
            slot: Slot(self.root),
            at: self.at,
        });
        match self.key {
            Some(key) => el.key(key).into_element(),
            None => el.into_element(),
        }
    }
}
