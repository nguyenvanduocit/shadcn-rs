//! Spinner (`ui/spinner`): the `loader-circle` icon turning once a second
//! (`size-4 animate-spin`).

use gui::prelude::*;
use gui::{Animation, Icon, IconElement, icon};

/// A turning loader; resize it as Vue's `class` does, `spinner().size(24.0)`.
#[must_use]
pub fn spinner() -> IconElement {
    spinner_of(Icon::LoaderCircle)
}

/// A custom spinner: `glyph` turning as [`spinner`]'s does
/// (`<LoaderIcon class="size-4 animate-spin" />`).
#[must_use]
pub fn spinner_of(glyph: Icon) -> IconElement {
    icon(glyph).named("spinner").animation(Animation::spin())
}
