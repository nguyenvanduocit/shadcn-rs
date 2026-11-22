//! The named steps of a theme that are not colours: scheme, radius, fonts, type scale, shadows, menu and icons.

use gui::Color;
use gui::style::{FontFamily, GenericFamily, Shadow, Shadows};

/// Light or dark: which [`super::Palette`] of a theme is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

impl Scheme {
    /// The other scheme.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

/// The radius options of shadcn's theme picker (shadcn-vue `registry/config.ts:63-69`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Radius {
    /// The base colour's own `--radius`: 0.625rem for all seven.
    #[default]
    Default,
    None,
    /// 0.45rem.
    Small,
    /// 0.625rem.
    Medium,
    /// 0.875rem.
    Large,
}

/// One CSS `rem`, in logical pixels.
pub const REM: f32 = 16.0;

/// Every base colour's `--radius`: 0.625rem.
pub(super) const BASE_RADIUS: f32 = 0.625 * REM;

impl Radius {
    /// `--radius` in logical pixels.
    #[must_use]
    pub const fn px(self) -> f32 {
        match self {
            Self::Default | Self::Medium => BASE_RADIUS,
            Self::None => 0.0,
            Self::Small => 0.45 * REM,
            Self::Large => 0.875 * REM,
        }
    }
}

/// A step of the radius scale, as shadcn's `@theme inline` derives it from `--radius`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RadiusStep {
    /// `--radius-sm`, × 0.6.
    Sm,
    /// `--radius-md`, × 0.8.
    Md,
    /// `--radius-lg`, × 1.
    Lg,
    /// `--radius-xl`, × 1.4.
    Xl,
    /// `--radius-2xl`, × 1.8.
    Xl2,
    /// `--radius-3xl`, × 2.2.
    Xl3,
    /// `--radius-4xl`, × 2.6.
    Xl4,
}

impl RadiusStep {
    /// Its factor of `--radius`.
    #[must_use]
    pub const fn factor(self) -> f32 {
        match self {
            Self::Sm => 0.6,
            Self::Md => 0.8,
            Self::Lg => 1.0,
            Self::Xl => 1.4,
            Self::Xl2 => 1.8,
            Self::Xl3 => 2.2,
            Self::Xl4 => 2.6,
        }
    }

    /// The step's radius for a theme radius of `radius` px.
    #[must_use]
    pub fn of(self, radius: f32) -> f32 {
        radius * self.factor()
    }
}

/// How a heading's family is chosen (shadcn's `fontHeading`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Heading {
    /// `inherit`: the sans family.
    #[default]
    AsSans,
    Family(FontFamily),
}

/// The theme's fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fonts {
    /// The body font, `--font-sans` (shadcn's `font`).
    pub sans: FontFamily,
    /// `--font-heading`.
    pub heading: Heading,
    /// `--font-mono`. shadcn has no mono option (Tailwind's stack stands); Geist Mono, as the verdict's harness
    /// sets it (`crates/shadcn/verdict/style.css`).
    pub mono: FontFamily,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            sans: FontFamily::INTER,
            heading: Heading::AsSans,
            mono: FontFamily::GEIST_MONO,
        }
    }
}

impl Fonts {
    /// The heading's family.
    #[must_use]
    pub const fn heading(&self) -> FontFamily {
        match self.heading {
            Heading::AsSans => self.sans,
            Heading::Family(f) => f,
        }
    }
}

/// The fonts of shadcn-vue's picker (`apps/v4/lib/fonts.ts`), by the family names that file writes: a font's real
/// family, not fontsource's `… Variable` alias of the CLI's list. Inter and Geist Mono are the ones `gui` bundles; an
/// app that picks another registers its files with `gui`'s fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    GeistSans,
    Inter,
    NotoSans,
    NunitoSans,
    Figtree,
    Roboto,
    Raleway,
    DmSans,
    PublicSans,
    Outfit,
    Oxanium,
    Manrope,
    SpaceGrotesk,
    Montserrat,
    IbmPlexSans,
    SourceSans3,
    InstrumentSans,
    JetbrainsMono,
    GeistMono,
    NotoSerif,
    RobotoSlab,
    Merriweather,
    Lora,
    PlayfairDisplay,
}

const fn family(names: &'static [&'static str], generic: GenericFamily) -> FontFamily {
    FontFamily::new(names, generic)
}

impl Font {
    pub const ALL: [Self; 24] = [
        Self::GeistSans,
        Self::Inter,
        Self::NotoSans,
        Self::NunitoSans,
        Self::Figtree,
        Self::Roboto,
        Self::Raleway,
        Self::DmSans,
        Self::PublicSans,
        Self::Outfit,
        Self::Oxanium,
        Self::Manrope,
        Self::SpaceGrotesk,
        Self::Montserrat,
        Self::IbmPlexSans,
        Self::SourceSans3,
        Self::InstrumentSans,
        Self::JetbrainsMono,
        Self::GeistMono,
        Self::NotoSerif,
        Self::RobotoSlab,
        Self::Merriweather,
        Self::Lora,
        Self::PlayfairDisplay,
    ];

    /// Its name in upstream's registry.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::GeistSans => "geist-sans",
            Self::Inter => "inter",
            Self::NotoSans => "noto-sans",
            Self::NunitoSans => "nunito-sans",
            Self::Figtree => "figtree",
            Self::Roboto => "roboto",
            Self::Raleway => "raleway",
            Self::DmSans => "dm-sans",
            Self::PublicSans => "public-sans",
            Self::Outfit => "outfit",
            Self::Oxanium => "oxanium",
            Self::Manrope => "manrope",
            Self::SpaceGrotesk => "space-grotesk",
            Self::Montserrat => "montserrat",
            Self::IbmPlexSans => "ibm-plex-sans",
            Self::SourceSans3 => "source-sans-3",
            Self::InstrumentSans => "instrument-sans",
            Self::JetbrainsMono => "jetbrains-mono",
            Self::GeistMono => "geist-mono",
            Self::NotoSerif => "noto-serif",
            Self::RobotoSlab => "roboto-slab",
            Self::Merriweather => "merriweather",
            Self::Lora => "lora",
            Self::PlayfairDisplay => "playfair-display",
        }
    }

    /// Its font stack.
    #[must_use]
    pub const fn family(self) -> FontFamily {
        use GenericFamily::{Monospace, SansSerif, Serif};
        match self {
            Self::GeistSans => family(&["Geist"], SansSerif),
            Self::Inter => FontFamily::INTER,
            Self::NotoSans => family(&["Noto Sans"], SansSerif),
            Self::NunitoSans => family(&["Nunito Sans"], SansSerif),
            Self::Figtree => family(&["Figtree"], SansSerif),
            Self::Roboto => family(&["Roboto"], SansSerif),
            Self::Raleway => family(&["Raleway"], SansSerif),
            Self::DmSans => family(&["DM Sans"], SansSerif),
            Self::PublicSans => family(&["Public Sans"], SansSerif),
            Self::Outfit => family(&["Outfit"], SansSerif),
            Self::Oxanium => family(&["Oxanium"], SansSerif),
            Self::Manrope => family(&["Manrope"], SansSerif),
            Self::SpaceGrotesk => family(&["Space Grotesk"], SansSerif),
            Self::Montserrat => family(&["Montserrat"], SansSerif),
            Self::IbmPlexSans => family(&["IBM Plex Sans"], SansSerif),
            Self::SourceSans3 => family(&["Source Sans 3"], SansSerif),
            Self::InstrumentSans => family(&["Instrument Sans"], SansSerif),
            Self::JetbrainsMono => family(&["JetBrains Mono"], Monospace),
            Self::GeistMono => FontFamily::GEIST_MONO,
            Self::NotoSerif => family(&["Noto Serif"], Serif),
            Self::RobotoSlab => family(&["Roboto Slab"], Serif),
            Self::Merriweather => family(&["Merriweather"], Serif),
            Self::Lora => family(&["Lora"], Serif),
            Self::PlayfairDisplay => family(&["Playfair Display"], Serif),
        }
    }
}

/// A step of the type scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextStep {
    Xs,
    Sm,
    Base,
    Lg,
    Xl,
    Xl2,
    Xl3,
    Xl4,
    Xl5,
    Xl6,
    Xl7,
    Xl8,
    Xl9,
}

/// A font size and its line height, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStep {
    pub size: f32,
    pub line_height: f32,
}

const fn step(size_rem: f32, line_height_rem: f32) -> TypeStep {
    TypeStep {
        size: size_rem * REM,
        line_height: line_height_rem * REM,
    }
}

/// Letter spacing steps, in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackingScale {
    pub tighter: f32,
    pub tight: f32,
    pub normal: f32,
    pub wide: f32,
    pub wider: f32,
    pub widest: f32,
}

/// Line-height steps, as multiples of the font size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeadingScale {
    pub tight: f32,
    pub snug: f32,
    pub normal: f32,
    pub relaxed: f32,
    pub loose: f32,
}

/// The type scale: Tailwind's (`theme.css`), which no shadcn theme changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    pub xs: TypeStep,
    pub sm: TypeStep,
    pub base: TypeStep,
    pub lg: TypeStep,
    pub xl: TypeStep,
    pub xl2: TypeStep,
    pub xl3: TypeStep,
    pub xl4: TypeStep,
    pub xl5: TypeStep,
    pub xl6: TypeStep,
    pub xl7: TypeStep,
    pub xl8: TypeStep,
    pub xl9: TypeStep,
    pub tracking: TrackingScale,
    pub leading: LeadingScale,
}

impl TypeScale {
    /// Tailwind 4's.
    pub const TAILWIND: Self = Self {
        xs: step(0.75, 1.0),
        sm: step(0.875, 1.25),
        base: step(1.0, 1.5),
        lg: step(1.125, 1.75),
        xl: step(1.25, 1.75),
        xl2: step(1.5, 2.0),
        xl3: step(1.875, 2.25),
        xl4: step(2.25, 2.5),
        xl5: step(3.0, 3.0),
        xl6: step(3.75, 3.75),
        xl7: step(4.5, 4.5),
        xl8: step(6.0, 6.0),
        xl9: step(8.0, 8.0),
        tracking: TrackingScale {
            tighter: -0.05,
            tight: -0.025,
            normal: 0.0,
            wide: 0.025,
            wider: 0.05,
            widest: 0.1,
        },
        leading: LeadingScale {
            tight: 1.25,
            snug: 1.375,
            normal: 1.5,
            relaxed: 1.625,
            loose: 2.0,
        },
    };

    /// The step `s`.
    #[must_use]
    pub const fn get(&self, s: TextStep) -> TypeStep {
        match s {
            TextStep::Xs => self.xs,
            TextStep::Sm => self.sm,
            TextStep::Base => self.base,
            TextStep::Lg => self.lg,
            TextStep::Xl => self.xl,
            TextStep::Xl2 => self.xl2,
            TextStep::Xl3 => self.xl3,
            TextStep::Xl4 => self.xl4,
            TextStep::Xl5 => self.xl5,
            TextStep::Xl6 => self.xl6,
            TextStep::Xl7 => self.xl7,
            TextStep::Xl8 => self.xl8,
            TextStep::Xl9 => self.xl9,
        }
    }
}

impl Default for TypeScale {
    fn default() -> Self {
        Self::TAILWIND
    }
}

/// A step of the shadow scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowStep {
    None,
    Xs2,
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
    Xl2,
}

/// The box-shadow scale: Tailwind's (`theme.css`), which no shadcn theme changes. Each layer is black with alpha, in
/// both schemes.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowScale {
    pub xs2: Shadows,
    pub xs: Shadows,
    pub sm: Shadows,
    pub md: Shadows,
    pub lg: Shadows,
    pub xl: Shadows,
    pub xl2: Shadows,
}

/// A black layer `(x, y, blur, spread)` at `alpha`.
const fn layer(x: f32, y: f32, blur: f32, spread: f32, alpha: f32) -> Shadow {
    Shadow {
        x,
        y,
        blur,
        spread,
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: alpha,
        },
    }
}

impl ShadowScale {
    /// Tailwind 4's.
    #[must_use]
    pub fn tailwind() -> Self {
        Self {
            xs2: Shadows::from([layer(0.0, 1.0, 0.0, 0.0, 0.05)]),
            xs: Shadows::from([layer(0.0, 1.0, 2.0, 0.0, 0.05)]),
            sm: Shadows::from([layer(0.0, 1.0, 3.0, 0.0, 0.1), layer(0.0, 1.0, 2.0, -1.0, 0.1)]),
            md: Shadows::from([layer(0.0, 4.0, 6.0, -1.0, 0.1), layer(0.0, 2.0, 4.0, -2.0, 0.1)]),
            lg: Shadows::from([layer(0.0, 10.0, 15.0, -3.0, 0.1), layer(0.0, 4.0, 6.0, -4.0, 0.1)]),
            xl: Shadows::from([layer(0.0, 20.0, 25.0, -5.0, 0.1), layer(0.0, 8.0, 10.0, -6.0, 0.1)]),
            xl2: Shadows::from([layer(0.0, 25.0, 50.0, -12.0, 0.25)]),
        }
    }

    /// The layers of `s`; none for [`ShadowStep::None`].
    #[must_use]
    pub fn get(&self, s: ShadowStep) -> Shadows {
        match s {
            ShadowStep::None => Shadows::new(),
            ShadowStep::Xs2 => self.xs2.clone(),
            ShadowStep::Xs => self.xs.clone(),
            ShadowStep::Sm => self.sm.clone(),
            ShadowStep::Md => self.md.clone(),
            ShadowStep::Lg => self.lg.clone(),
            ShadowStep::Xl => self.xl.clone(),
            ShadowStep::Xl2 => self.xl2.clone(),
        }
    }
}

impl Default for ShadowScale {
    fn default() -> Self {
        Self::tailwind()
    }
}

/// shadcn's `menuAccent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MenuAccent {
    /// The palette as it is.
    #[default]
    Subtle,
    /// Accent tokens take the primary ones ([`super::Palette::bold`]).
    Bold,
}

/// shadcn's menu options. `menuColor`'s four values are the two independent flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Menu {
    pub accent: MenuAccent,
    /// Menu content shows the dark scheme to its subtree in both app schemes, as upstream's `.dark` on
    /// `cn-menu-target` does: it provides `with_theme(|t| t.scheme = Scheme::Dark, …)`.
    pub inverted: bool,
    /// Menu content paints a translucent popover over a blurred backdrop, and its items a translucent highlight
    /// (`.cn-menu-translucent`, the same in all eight styles, so written once in the menu components).
    pub translucent: bool,
}
