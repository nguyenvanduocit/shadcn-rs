//! The focus ring every focusable shadcn control shows:
//! `focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]`.

use gui::interact::Flag;
use gui::prelude::*;
use gui::{Color, Style};

use crate::theme::Colors;

/// `s` with the focus ring of `t`.
#[must_use]
pub fn focus_ring(s: Style, t: &Colors) -> Style {
    s.border_color(t.ring).ring(Ring::new(3.0, t.ring.opacity(0.5)))
}

/// The destructive focus ring (`focus-visible:ring-destructive/20`, `/40` in dark).
#[must_use]
pub fn destructive_ring(s: Style, t: &Colors) -> Style {
    let alpha = if t.dark { 0.4 } else { 0.2 };
    s.border_color(t.ring)
        .ring(Ring::new(3.0, t.destructive.opacity(alpha)))
}

/// What a state adds to a node, as bindings of the properties it changes:
/// each closure reads the node's [`Interaction`] where the node resolves,
/// so only that node's style runs again when the flag changes. Outside the
/// state a property keeps the fixed value written before the binding (the
/// component's, or a caller's restyle of it).
pub trait StateLook: Styled {
    /// The border colour and ring of `ring` (a style [`focus_ring`] or
    /// [`destructive_ring`] made over `Style::default()`) while `shown`
    /// holds (`move || i.focus_visible()`): what `focus-visible:` adds.
    #[must_use]
    fn ring_while(mut self, shown: impl Fn() -> bool + Clone + 'static, ring: &Style) -> Self {
        let (on, glow) = (ring.border_color, ring.ring);
        let (border, off) = {
            let s = self.style_mut();
            (s.border_color, s.ring)
        };
        let again = shown.clone();
        self.border_color(move || if shown() { on } else { border })
            .ring(move || if again() { glow } else { off })
    }

    /// [`focus_ring`] while `i` shows focus, read through a source (no tracker per binding).
    #[must_use]
    fn focus_ring_on(mut self, i: Interaction, t: &Colors) -> Self {
        let ring = focus_ring(Style::default(), t);
        let (border, off) = {
            let s = self.style_mut();
            (s.border_color, s.ring)
        };
        self.border_color(i.when(Flag::FocusVisible, border, ring.border_color))
            .ring(i.when(Flag::FocusVisible, off, ring.ring))
    }

    /// The background `color` while `on` holds (`move || i.hovered()`).
    #[must_use]
    fn background_while(mut self, on: impl Fn() -> bool + 'static, color: impl Into<Option<Color>>) -> Self {
        let (base, color) = (self.style_mut().background, color.into());
        self.background(move || if on() { color } else { base })
    }

    /// The text colour `color` while `on` holds.
    #[must_use]
    fn text_color_while(mut self, on: impl Fn() -> bool + 'static, color: impl Into<Option<Color>>) -> Self {
        let (base, color) = (self.style_mut().content.color, color.into());
        self.text_color(move || if on() { color } else { base })
    }

    /// `opacity` while `i` is disabled (`disabled:opacity-50`).
    #[must_use]
    fn dim_disabled(mut self, i: Interaction, opacity: f32) -> Self {
        let base = self.style_mut().opacity;
        self.opacity(i.when(Flag::Disabled, base, opacity))
    }

    /// The not-allowed cursor while `i` is disabled (`disabled:cursor-not-allowed`).
    #[must_use]
    fn cursor_disabled(mut self, i: Interaction) -> Self {
        let base = self.style_mut().cursor;
        self.cursor(i.when(Flag::Disabled, base, Some(Cursor::NotAllowed)))
    }
}

impl<S: Styled> StateLook for S {}
