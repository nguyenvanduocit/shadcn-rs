//! The reading direction a primitive navigates by (reka's `useDirection` and
//! `ConfigProvider`'s `dir`): the primitive's own when the caller sets one,
//! else the nearest provided above it, else left to right.
//!
//! It decides which of ArrowLeft and ArrowRight goes forward
//! ([`crate::keymap`]); Up, Down, Home and End do not turn.
//!
//! - **Read when a key arrives.** [`use_direction`] gives a [`ReadDirection`]
//!   (`Copy`), which a primitive keeps from its setup and reads at each key,
//!   so an app that switches direction at run time turns every keymap with
//!   it, rendering nothing.
//! - **One value for layout and keys.** Layout's direction
//!   (`gui::Style::direction`, ADR-0004) flows to every node by itself and
//!   knows nothing of this context. A subtree is made right to left by one
//!   value, a `Signal<Direction>`, given to both halves: [`provide_direction`]
//!   in the setup that owns it, which returns a [`DirectionScope`], and
//!   [`DirectionScope::root`] on that subtree's root node, which binds its
//!   layout direction to the same signal. So the two cannot disagree.

use gui::prelude::*;
use gui::{Direction, Div, Signal};

/// The direction provided to a subtree.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Provided(Signal<Direction>);

/// A subtree's direction, as [`provide_direction`] gave it: its root node
/// lays out by the same value ([`DirectionScope::root`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionScope(Signal<Direction>);

impl DirectionScope {
    /// `d`, the subtree's root, laid out in the provided direction (bound
    /// to the node: a switch lays it out again, renders nothing).
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let direction = self.0;
        d.direction(move || direction.get())
    }
}

/// Give the primitives below the component calling this (in its setup)
/// `direction` (reka's `<ConfigProvider dir>`); lay the subtree's root out
/// by it with the returned scope's [`DirectionScope::root`].
pub fn provide_direction(direction: Signal<Direction>) -> DirectionScope {
    gui::provide(Provided(direction));
    DirectionScope(direction)
}

/// The direction a primitive navigates by, kept from its setup and read
/// when a key arrives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadDirection {
    own: Option<Direction>,
    provided: Option<Signal<Direction>>,
}

impl ReadDirection {
    /// A fixed direction (a primitive's own `dir`).
    #[must_use]
    pub const fn fixed(direction: Direction) -> Self {
        Self {
            own: Some(direction),
            provided: None,
        }
    }

    /// The direction now (untracked: keys read it, nothing renders by it).
    #[must_use]
    pub fn get(self) -> Direction {
        self.own
            .or_else(|| self.provided.filter(Signal::is_alive).map(|s| s.get_untracked()))
            .unwrap_or_default()
    }
}

/// A fixed direction.
impl From<Direction> for ReadDirection {
    fn from(direction: Direction) -> Self {
        Self::fixed(direction)
    }
}

/// The direction a primitive navigates by: `own` (the primitive's `dir`),
/// else the nearest provided, else [`Direction::Ltr`]. Called in setup.
#[must_use]
pub fn use_direction(own: Option<Direction>) -> ReadDirection {
    ReadDirection {
        own,
        provided: gui::use_context::<Provided>().map(|p| p.0),
    }
}

#[cfg(test)]
mod tests {
    use gui::{Direction, Signal};

    use super::{provide_direction, use_direction};

    #[test]
    fn own_then_provided_then_left_to_right_read_when_asked() {
        assert_eq!(use_direction(None).get(), Direction::Ltr);
        assert_eq!(use_direction(Some(Direction::Rtl)).get(), Direction::Rtl);
        // Outside any component, a provided value is the application's.
        let app = Signal::new(Direction::Rtl);
        provide_direction(app);
        let read = use_direction(None);
        assert_eq!(read.get(), Direction::Rtl);
        assert_eq!(use_direction(Some(Direction::Ltr)).get(), Direction::Ltr);
        app.set(Direction::Ltr);
        assert_eq!(
            read.get(),
            Direction::Ltr,
            "a switch at run time reaches what was read in setup"
        );
    }
}
