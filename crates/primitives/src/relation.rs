//! A trigger tied to the content it opens, by typed relations instead of
//! string ids (reka's `useTriggerId`, `useContentId` and `useId`).
//!
//! The trigger points at its content (`aria-controls`, or
//! `aria-describedby` for a tooltip) only while the content is open; the
//! engine sends a relation only towards a node present in the tree, in the
//! same update the node comes or goes (ADR-0024), so a reference never
//! names a node that is not there, and content kept mounted while closed is
//! not named either. The content is named by its trigger
//! (`aria-labelledby`) where the primitive says so (a dialog by its title,
//! a menu by its trigger). No id is a string, so a caller cannot set one
//! that clashes.

use gui::{Div, NodeRef};

/// What a trigger's relation to its content is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Points {
    /// It opens and controls the content (`aria-controls`): a menu, a
    /// popover, a dialog, a select's list.
    Controls,
    /// The content describes it (`aria-describedby`): a tooltip.
    DescribedBy,
}

/// The pair of nodes a trigger and its content are: made once in the root's
/// setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Linked {
    pub trigger: NodeRef,
    pub content: NodeRef,
    pub points: Points,
}

impl Linked {
    #[must_use]
    pub fn new(points: Points) -> Self {
        Self {
            trigger: NodeRef::new(),
            content: NodeRef::new(),
            points,
        }
    }

    /// `d` as the trigger, pointing at the content while `open`.
    #[must_use]
    pub fn trigger(self, d: Div, open: bool) -> Div {
        let d = d.node_ref(self.trigger);
        if !open {
            return d;
        }
        match self.points {
            Points::Controls => d.controls(self.content),
            Points::DescribedBy => d.described_by(self.content),
        }
    }

    /// `d` as the content; `labelled` names it by its trigger.
    #[must_use]
    pub fn content(self, d: Div, labelled: bool) -> Div {
        let d = d.node_ref(self.content);
        if labelled { d.labelled_by(self.trigger) } else { d }
    }
}
