//! shadcn's theme (ADR-0022): the tokens every component reads, as typed values. A theme is the second axis beside
//! a style ([`crate::design::Design`]), chosen apart from it as upstream does.
//!
//! - Colours: the seven base colours and seventeen accents of shadcn-vue's `registry/themes.ts`, light and dark,
//!   kept in OKLCH ([`Palette`]).
//! - Named steps: radius ([`RadiusStep`] of `--radius`), type ([`TypeScale`]), shadows ([`ShadowScale`]), fonts.
//! - Menu options and the icon library.
//!
//! An app installs a theme and a style once ([`install`]); a subtree overrides either ([`with_theme`],
//! [`crate::design::with_design`]). Each is provided as one reactive group per field with equality cutoff, so a
//! binding reruns only when a group it read changed ([`ThemeHandle`]).

mod data;
pub(crate) mod handle;
mod ink;
mod palette;
mod scale;

use crate::icons::IconSet;
pub use handle::{Colors, TextStyle, ThemeHandle, install, use_theme, with_theme};
pub use ink::{Ink, StateTones, Tone};
pub use palette::{Accent, BaseColor, ColorRole, Palette, Palettes};

pub use scale::{
    Font, Fonts, Heading, LeadingScale, Menu, MenuAccent, REM, Radius, RadiusStep, Scheme, ShadowScale, ShadowStep,
    TextStep, TrackingScale, TypeScale, TypeStep,
};

/// A theme: every token a component reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub scheme: Scheme,
    /// The light and dark palettes, as the base colour and accent write them; [`Menu::accent`] applies when read.
    pub colors: Palettes,
    /// `--radius` in logical pixels; the steps are fractions and multiples of it ([`RadiusStep`]).
    pub radius: f32,
    pub fonts: Fonts,
    pub type_scale: TypeScale,
    pub shadows: ShadowScale,
    pub menu: Menu,
    /// The icon library, one of those this build carries (each a cargo feature).
    pub icons: IconSet,
}

impl Theme {
    /// A base colour's theme, light, with every other option at upstream's default.
    #[must_use]
    pub fn new(base: BaseColor) -> Self {
        Self {
            scheme: Scheme::Light,
            colors: base.palettes(),
            radius: Radius::Default.px(),
            fonts: Fonts::default(),
            type_scale: TypeScale::TAILWIND,
            shadows: ShadowScale::tailwind(),
            menu: Menu::default(),
            icons: IconSet::default(),
        }
    }

    /// With `accent` written over its colours.
    #[must_use]
    pub fn accent(mut self, accent: Accent) -> Self {
        self.colors = accent.over(self.colors);
        self
    }

    /// In `scheme`.
    #[must_use]
    pub fn scheme(mut self, scheme: Scheme) -> Self {
        self.scheme = scheme;
        self
    }

    /// With the radius option `radius`.
    #[must_use]
    pub fn radius(mut self, radius: Radius) -> Self {
        self.radius = radius.px();
        self
    }

    /// With `font` as the body font.
    #[must_use]
    pub fn font(mut self, font: Font) -> Self {
        self.fonts.sans = font.family();
        self
    }

    /// With `font` for headings; the body font's own is `inherit` (`registry/config.ts:386-387`).
    #[must_use]
    pub fn heading(mut self, font: Font) -> Self {
        let family = font.family();
        self.fonts.heading = if family == self.fonts.sans {
            Heading::AsSans
        } else {
            Heading::Family(family)
        };
        self
    }

    /// With the menu options `menu`.
    #[must_use]
    pub fn menu(mut self, menu: Menu) -> Self {
        self.menu = menu;
        self
    }

    /// Drawing icons from `icons`.
    #[must_use]
    pub fn icons(mut self, icons: IconSet) -> Self {
        self.icons = icons;
        self
    }

    /// Both palettes as read: with [`MenuAccent::Bold`] applied when chosen.
    #[must_use]
    pub fn palettes(&self) -> Palettes {
        match self.menu.accent {
            MenuAccent::Subtle => self.colors,
            MenuAccent::Bold => self.colors.bold(),
        }
    }

    /// The palette of the theme's scheme, as read.
    #[must_use]
    pub fn palette(&self) -> Palette {
        *self.palettes().get(self.scheme)
    }
}

impl Default for Theme {
    /// Neutral, light.
    fn default() -> Self {
        Self::new(BaseColor::Neutral)
    }
}

/// The painted palette of the scheme in scope, read in render: what components read today (`let t = theme();
/// t.border`). A render that calls it renders again when the scheme or the colours change. S8 moves every
/// component to bindings that resolve a [`Tone`] ([`ThemeHandle::ink`]) and removes this.
#[must_use]
pub fn theme() -> Colors {
    use_theme().colors()
}
