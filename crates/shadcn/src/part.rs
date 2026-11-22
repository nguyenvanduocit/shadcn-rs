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

use gui::{
    ComponentElement, Div, Element, ElementKey, Fluent, IntoElement, Slot, Style, Styled, TextField, UseModel, watch,
};

/// Called in setup: `field` and the text `value` (a part's `v-model`) stay
/// one. An edit writes the text up; a text written from outside (a form's
/// reset, a sibling sharing the model) is what the field then shows.
pub(crate) fn bind_field<P: 'static>(field: TextField, value: UseModel<P, String>) {
    watch(
        move || field.edit.with(|e| e.text.clone()),
        move |text, _| value.set(text.clone()),
    );
    watch(
        move || value.get(),
        move |text, _| {
            if field.edit.with_untracked(|e| e.text != *text) {
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
}

impl<P: 'static> Part<P> {
    /// A part whose box starts as `root`, its instance made by `mount`
    /// (a call of [`gui::component`]).
    #[must_use]
    pub fn new(props: P, root: Div, mount: fn(PartProps<P>) -> ComponentElement) -> Self {
        Self {
            props,
            root,
            key: None,
            mount,
        }
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

    /// Called on a click on the box (a part whose own click does something
    /// else replaces this one).
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
    fn style_mut(&mut self) -> &mut Style {
        self.root.style_mut()
    }
}

impl<P: 'static> IntoElement for Part<P> {
    fn into_element(self) -> Element {
        let el = (self.mount)(PartProps {
            props: self.props,
            slot: Slot(self.root),
        });
        match self.key {
            Some(key) => el.key(key).into_element(),
            None => el.into_element(),
        }
    }
}
