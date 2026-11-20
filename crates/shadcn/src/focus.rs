//! The focus ring every focusable shadcn control shows:
//! `focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]`.

use gui::{Style, Styled};

use crate::theme::Theme;

/// `s` with the focus ring of `t`.
#[must_use]
pub fn focus_ring(s: Style, t: &Theme) -> Style {
    s.border_color(t.ring).ring(3.0, t.ring.opacity(0.5))
}

/// The destructive focus ring (`focus-visible:ring-destructive/20`, `/40` in dark).
#[must_use]
pub fn destructive_ring(s: Style, t: &Theme) -> Style {
    let alpha = if t.dark { 0.4 } else { 0.2 };
    s.border_color(t.ring).ring(3.0, t.destructive.opacity(alpha))
}
