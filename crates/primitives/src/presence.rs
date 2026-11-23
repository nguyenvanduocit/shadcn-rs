//! Whether a part is there (reka's `Presence`): open, kept mounted while
//! closed, or gone, and the four stages a caller observes on the way.
//!
//! The exit animation is the engine's: a subtree that leaves the tree stays
//! painted, inert, until its exit animation ends (`gui::motion`, ADR-0007),
//! and it leaves the layer stack and the focus scopes as its exit begins
//! (ADR-0024). So the part's state reads closed as soon as it is asked to
//! close, its look picks the exit animation from that, and nothing in it
//! runs while it plays out. Shown again before the exit ends, it is the same
//! instance with its state.
//!
//! Content the caller keeps mounted while closed (reka's `forceMount`)
//! stays in the tree hidden and inert: no layer, no focus, no pointer, not
//! read, its state kept; it plays its entry animation once, when it is
//! first shown.
//!
//! **Stages** ([`gui::Stage`], reka's `enter`, `after-enter`, `leave`,
//! `after-leave`): [`Present`] tells its caller each one the part reaches,
//! in order, none skipped. The engine reports them from the part's own
//! node (`gui::Div::on_stage`), which knows when its own animations start
//! and end, its children's aside, under reduced motion too: none is
//! guessed from a computed style as reka must. A part kept mounted does not
//! leave the tree, so its closing and showing again are told by
//! [`Present`] itself: closed, it is hidden at once (`Exiting` then
//! `Exited`); shown again, it plays no second entry (`Entering` then
//! `Entered`). reka's documented quirks are not kept: `Exiting` is told
//! whether or not the exit animation's name differs from the entry's.

use gui::Stage;
use gui::prelude::*;
use gui::{Div, Element};

/// Where a part is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Presence {
    /// Open: shown, live.
    Open,
    /// Closed, kept mounted (`forceMount`): hidden and inert, its state kept.
    Kept,
    /// Closed and gone (its exit, if it has one, plays on its own).
    Gone,
}

impl Presence {
    /// The presence of a part that is `open`, kept mounted while closed when
    /// `keep_mounted`.
    #[must_use]
    pub const fn of(open: bool, keep_mounted: bool) -> Self {
        match (open, keep_mounted) {
            (true, _) => Self::Open,
            (false, true) => Self::Kept,
            (false, false) => Self::Gone,
        }
    }

    /// Whether the part is open: what its look reads (entry or exit
    /// animation, `data-state`).
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }

    /// `d` as this presence makes it.
    #[must_use]
    pub fn render(self, d: Div) -> Element {
        match self {
            Self::Open => d.into_element(),
            Self::Kept => d.invisible(true).inert(true).into_element(),
            Self::Gone => Element::Empty,
        }
    }
}

/// A part's presence over time, made once in its setup: it tells
/// `on_stage` each [`Stage`] the part reaches and holds the last (tracked).
#[derive(Clone, Copy, PartialEq)]
pub struct Present {
    stage: gui::Signal<Option<Stage>>,
    presence: gui::Signal<Presence>,
    tell: gui::Signal<gui::Callback<Stage>>,
}

impl std::fmt::Debug for Present {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Present").finish_non_exhaustive()
    }
}

impl Present {
    /// A part whose presence `presence` gives (read tracked, in setup's
    /// scope), telling `on_stage` its stages.
    #[must_use]
    #[track_caller]
    pub fn new(presence: impl Fn() -> Presence + 'static, on_stage: impl Into<gui::Callback<Stage>>) -> Self {
        let now = gui::untracked(&presence);
        let tell: gui::Callback<Stage> = on_stage.into();
        let me = Self {
            stage: gui::Signal::labeled("presence stage", None),
            presence: gui::Signal::labeled("presence", now),
            tell: gui::Signal::new(tell),
        };
        gui::watch(presence, move |new, old| {
            me.presence.set(*new);
            // A kept part never leaves the tree: its stages are told here.
            match (*old, *new) {
                (Presence::Open, Presence::Kept) => {
                    me.reach(Stage::Exiting);
                    me.reach(Stage::Exited);
                }
                (Presence::Kept, Presence::Open) => {
                    me.reach(Stage::Entering);
                    me.reach(Stage::Entered);
                }
                _ => {}
            }
        });
        me
    }

    /// The last stage reached (tracked); none before the part is first laid
    /// out.
    #[must_use]
    pub fn stage(self) -> Option<Stage> {
        self.stage.get()
    }

    fn reach(self, stage: Stage) {
        if !self.stage.is_alive() {
            return;
        }
        self.stage.set(Some(stage));
        let tell = self.tell.get_untracked();
        tell.call(stage);
    }

    /// `d` as `presence` (the value [`Present::new`] reads, now) makes it
    /// ([`Presence::render`]), its node reporting its stages. A kept part's
    /// node is laid out while hidden; what it reports then is not a stage
    /// of the part.
    #[must_use]
    pub fn render(self, presence: Presence, d: Div) -> Element {
        presence.render(d.on_stage(move |stage| {
            let kept = self.presence.is_alive() && self.presence.get_untracked() == Presence::Kept;
            if !kept || matches!(stage, Stage::Exiting | Stage::Exited) {
                self.reach(stage);
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::Presence;

    #[test]
    fn open_kept_or_gone() {
        assert_eq!(Presence::of(true, false), Presence::Open);
        assert_eq!(Presence::of(true, true), Presence::Open);
        assert_eq!(Presence::of(false, true), Presence::Kept);
        assert_eq!(Presence::of(false, false), Presence::Gone);
        assert!(!Presence::Kept.is_open());
    }
}
