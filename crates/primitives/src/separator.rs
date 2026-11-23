//! A rule between groups of content (reka's `Separator`).
//!
//! - Semantic: role `separator` (AccessKit's `Splitter`, as Chromium maps
//!   it); assistive technology reads it as a boundary. A vertical one says
//!   so (`aria-orientation="vertical"`); a horizontal one is left to ARIA's
//!   default, as reka leaves it.
//! - Decorative: nothing to assistive technology (role `none`); shadcn's
//!   separator is decorative by default.
//! - Neither takes focus nor keys.
//!
//! The look reads the orientation (a horizontal rule spans the width, a
//! vertical one the height). Menus' and selects' separators are their own
//! primitives' (reka gives `MenuSeparator` a horizontal `separator` role and
//! hides `SelectSeparator`).

use gui::{Axis, Div, Role};

/// What a separator is to assistive technology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Separator {
    pub orientation: Axis,
    /// Shown only, not read (shadcn's default).
    pub decorative: bool,
}

impl Separator {
    /// A horizontal, decorative separator: shadcn's default.
    pub const DEFAULT: Self = Self {
        orientation: Axis::Horizontal,
        decorative: true,
    };

    #[must_use]
    pub const fn new(orientation: Axis, decorative: bool) -> Self {
        Self {
            orientation,
            decorative,
        }
    }

    /// `d` as this separator is to assistive technology.
    #[must_use]
    pub fn apply(self, d: Div) -> Div {
        if self.decorative {
            d.aria_hidden()
        } else {
            let d = d.role(Role::Splitter);
            match self.orientation {
                Axis::Vertical => d.aria_orientation(Axis::Vertical),
                Axis::Horizontal => d,
            }
        }
    }
}

impl Default for Separator {
    fn default() -> Self {
        Self::DEFAULT
    }
}
