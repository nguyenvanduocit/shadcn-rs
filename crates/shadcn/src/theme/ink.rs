//! Colours as data: an [`Ink`] names a token (or the enclosing content colour), a [`Tone`] holds one per scheme, and
//! [`StateTones`] one per state a control shows. A theme resolves them where a binding runs.

use gui::Color;
use gui::color::Oklch;

use super::palette::{ColorRole, Palette};
use super::scale::Scheme;

/// A colour as data, resolved against the palette in scope when the binding that reads it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    /// A token: `bg-primary`.
    Role(ColorRole),
    /// A token with its alpha multiplied: `bg-primary/80`.
    Alpha(ColorRole, f32),
    /// `color-mix(in oklch, <first>, <second> <t × 100%>)`.
    Mix(ColorRole, ColorRole, f32),
    /// The colour the enclosing container gives its content (`currentColor`).
    Content,
    /// A colour of its own: an app's, or upstream's `bg-black/10`.
    Fixed(Color),
    /// `transparent`.
    Clear,
}

/// Transparent black, as CSS's `transparent`.
fn transparent() -> Oklch {
    Oklch::new(0.0, 0.0, 0.0).with_alpha(0.0)
}

impl Ink {
    /// The colour against `palette`, with `content` as the enclosing content colour.
    #[must_use]
    pub fn resolve(self, palette: &Palette, content: Oklch) -> Oklch {
        match self {
            Self::Role(r) => palette.get(r),
            Self::Alpha(r, a) => palette.get(r).with_alpha(|x: f32| x * a),
            Self::Mix(a, b, t) => palette.get(a).mix(palette.get(b), t),
            Self::Content => content,
            Self::Fixed(c) => Oklch::from(c),
            Self::Clear => transparent(),
        }
    }
}

/// One ink per scheme. Upstream's `dark:` is a style's decision, so it is data, not a branch in a component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub light: Ink,
    pub dark: Ink,
}

impl Tone {
    /// The same ink in both schemes.
    #[must_use]
    pub const fn of(ink: Ink) -> Self {
        Self { light: ink, dark: ink }
    }

    /// A token in both schemes.
    #[must_use]
    pub const fn role(role: ColorRole) -> Self {
        Self::of(Ink::Role(role))
    }

    /// `transparent` in both schemes.
    pub const CLEAR: Self = Self::of(Ink::Clear);

    /// The ink of `scheme`.
    #[must_use]
    pub const fn get(self, scheme: Scheme) -> Ink {
        match scheme {
            Scheme::Light => self.light,
            Scheme::Dark => self.dark,
        }
    }
}

/// A tone for each combination of the two states that change a control's fill and text, hover and expanded: what
/// Chrome shows in each, as data, so no rule of precedence is needed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateTones {
    pub rest: Tone,
    pub hovered: Tone,
    pub expanded: Tone,
    pub expanded_hovered: Tone,
}

impl StateTones {
    /// The same tone in every state.
    #[must_use]
    pub const fn of(tone: Tone) -> Self {
        Self {
            rest: tone,
            hovered: tone,
            expanded: tone,
            expanded_hovered: tone,
        }
    }

    /// Hover changes the tone; being expanded changes nothing.
    #[must_use]
    pub const fn hover(rest: Tone, hovered: Tone) -> Self {
        Self {
            rest,
            hovered,
            expanded: rest,
            expanded_hovered: hovered,
        }
    }

    /// Expanded shows as hovered (outline and ghost buttons).
    #[must_use]
    pub const fn hover_or_expanded(rest: Tone, hovered: Tone) -> Self {
        Self {
            rest,
            hovered,
            expanded: hovered,
            expanded_hovered: hovered,
        }
    }

    /// The tone of the state a binding reads.
    #[must_use]
    pub const fn get(&self, hovered: bool, expanded: bool) -> Tone {
        match (expanded, hovered) {
            (false, false) => self.rest,
            (false, true) => self.hovered,
            (true, false) => self.expanded,
            (true, true) => self.expanded_hovered,
        }
    }
}
