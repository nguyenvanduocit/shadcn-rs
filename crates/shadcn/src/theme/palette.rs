//! Palettes: every colour token of a shadcn theme in one scheme, and the base colours and accents that make them.

use gui::Color;
use gui::color::Oklch;

use super::data::{self, AccentData, Raw};

/// Every colour token of a shadcn theme in one scheme (`registry/themes.ts`'s `cssVars.light` or `.dark`), plus the
/// two the shadcn site adds for code (`--code`, `--code-foreground`, its `--surface` pair).
///
/// A theme keeps its tokens as [`Oklch`] (`Palette<Oklch>`, the default), as upstream writes them, so a relative
/// colour or a mix starts from the token itself, out-of-gamut chroma included. `Palette<Color>` is the same palette
/// painted ([`Palette::painted`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette<C = Oklch> {
    pub background: C,
    pub foreground: C,
    pub card: C,
    pub card_foreground: C,
    pub popover: C,
    pub popover_foreground: C,
    pub primary: C,
    pub primary_foreground: C,
    pub secondary: C,
    pub secondary_foreground: C,
    pub muted: C,
    pub muted_foreground: C,
    pub accent: C,
    pub accent_foreground: C,
    pub destructive: C,
    pub border: C,
    pub input: C,
    pub ring: C,
    /// `chart-1` … `chart-5`. shadcn-vue's charts follow the theme: its `chartColor` is in the schema but
    /// `buildRegistryTheme` never applies it (shadcn-ui copies another theme's chart steps), so there is no chart
    /// axis here.
    pub chart: [C; 5],
    pub sidebar: C,
    pub sidebar_foreground: C,
    pub sidebar_primary: C,
    pub sidebar_primary_foreground: C,
    pub sidebar_accent: C,
    pub sidebar_accent_foreground: C,
    pub sidebar_border: C,
    pub sidebar_ring: C,
    /// The shadcn site's `--code` (its `--surface`, `apps/v4/app/globals.css`): `oklch(0.98 0 0)`, dark
    /// `oklch(0.2 0 0)`, whatever the base colour.
    pub code: C,
    /// `--code-foreground`: the base's `foreground` in light, `oklch(0.708 0 0)` in dark.
    pub code_foreground: C,
}

/// A colour token by name: what an [`super::Ink`] refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorRole {
    Background,
    Foreground,
    Card,
    CardForeground,
    Popover,
    PopoverForeground,
    Primary,
    PrimaryForeground,
    Secondary,
    SecondaryForeground,
    Muted,
    MutedForeground,
    Accent,
    AccentForeground,
    Destructive,
    Border,
    Input,
    Ring,
    Chart1,
    Chart2,
    Chart3,
    Chart4,
    Chart5,
    Sidebar,
    SidebarForeground,
    SidebarPrimary,
    SidebarPrimaryForeground,
    SidebarAccent,
    SidebarAccentForeground,
    SidebarBorder,
    SidebarRing,
    Code,
    CodeForeground,
}

impl ColorRole {
    /// Every role, in upstream's order.
    pub const ALL: [Self; 33] = [
        Self::Background,
        Self::Foreground,
        Self::Card,
        Self::CardForeground,
        Self::Popover,
        Self::PopoverForeground,
        Self::Primary,
        Self::PrimaryForeground,
        Self::Secondary,
        Self::SecondaryForeground,
        Self::Muted,
        Self::MutedForeground,
        Self::Accent,
        Self::AccentForeground,
        Self::Destructive,
        Self::Border,
        Self::Input,
        Self::Ring,
        Self::Chart1,
        Self::Chart2,
        Self::Chart3,
        Self::Chart4,
        Self::Chart5,
        Self::Sidebar,
        Self::SidebarForeground,
        Self::SidebarPrimary,
        Self::SidebarPrimaryForeground,
        Self::SidebarAccent,
        Self::SidebarAccentForeground,
        Self::SidebarBorder,
        Self::SidebarRing,
        Self::Code,
        Self::CodeForeground,
    ];

    /// The CSS variable's name without `--`: `muted-foreground`, `chart-1`.
    #[must_use]
    pub const fn css_name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Foreground => "foreground",
            Self::Card => "card",
            Self::CardForeground => "card-foreground",
            Self::Popover => "popover",
            Self::PopoverForeground => "popover-foreground",
            Self::Primary => "primary",
            Self::PrimaryForeground => "primary-foreground",
            Self::Secondary => "secondary",
            Self::SecondaryForeground => "secondary-foreground",
            Self::Muted => "muted",
            Self::MutedForeground => "muted-foreground",
            Self::Accent => "accent",
            Self::AccentForeground => "accent-foreground",
            Self::Destructive => "destructive",
            Self::Border => "border",
            Self::Input => "input",
            Self::Ring => "ring",
            Self::Chart1 => "chart-1",
            Self::Chart2 => "chart-2",
            Self::Chart3 => "chart-3",
            Self::Chart4 => "chart-4",
            Self::Chart5 => "chart-5",
            Self::Sidebar => "sidebar",
            Self::SidebarForeground => "sidebar-foreground",
            Self::SidebarPrimary => "sidebar-primary",
            Self::SidebarPrimaryForeground => "sidebar-primary-foreground",
            Self::SidebarAccent => "sidebar-accent",
            Self::SidebarAccentForeground => "sidebar-accent-foreground",
            Self::SidebarBorder => "sidebar-border",
            Self::SidebarRing => "sidebar-ring",
            Self::Code => "code",
            Self::CodeForeground => "code-foreground",
        }
    }
}

impl<C: Copy> Palette<C> {
    /// The token `role` names.
    #[must_use]
    pub const fn get(&self, role: ColorRole) -> C {
        match role {
            ColorRole::Background => self.background,
            ColorRole::Foreground => self.foreground,
            ColorRole::Card => self.card,
            ColorRole::CardForeground => self.card_foreground,
            ColorRole::Popover => self.popover,
            ColorRole::PopoverForeground => self.popover_foreground,
            ColorRole::Primary => self.primary,
            ColorRole::PrimaryForeground => self.primary_foreground,
            ColorRole::Secondary => self.secondary,
            ColorRole::SecondaryForeground => self.secondary_foreground,
            ColorRole::Muted => self.muted,
            ColorRole::MutedForeground => self.muted_foreground,
            ColorRole::Accent => self.accent,
            ColorRole::AccentForeground => self.accent_foreground,
            ColorRole::Destructive => self.destructive,
            ColorRole::Border => self.border,
            ColorRole::Input => self.input,
            ColorRole::Ring => self.ring,
            ColorRole::Chart1 => self.chart[0],
            ColorRole::Chart2 => self.chart[1],
            ColorRole::Chart3 => self.chart[2],
            ColorRole::Chart4 => self.chart[3],
            ColorRole::Chart5 => self.chart[4],
            ColorRole::Sidebar => self.sidebar,
            ColorRole::SidebarForeground => self.sidebar_foreground,
            ColorRole::SidebarPrimary => self.sidebar_primary,
            ColorRole::SidebarPrimaryForeground => self.sidebar_primary_foreground,
            ColorRole::SidebarAccent => self.sidebar_accent,
            ColorRole::SidebarAccentForeground => self.sidebar_accent_foreground,
            ColorRole::SidebarBorder => self.sidebar_border,
            ColorRole::SidebarRing => self.sidebar_ring,
            ColorRole::Code => self.code,
            ColorRole::CodeForeground => self.code_foreground,
        }
    }

    /// The same palette with every token passed through `f`.
    #[must_use]
    pub fn map<D>(self, f: impl Fn(C) -> D) -> Palette<D> {
        Palette {
            background: f(self.background),
            foreground: f(self.foreground),
            card: f(self.card),
            card_foreground: f(self.card_foreground),
            popover: f(self.popover),
            popover_foreground: f(self.popover_foreground),
            primary: f(self.primary),
            primary_foreground: f(self.primary_foreground),
            secondary: f(self.secondary),
            secondary_foreground: f(self.secondary_foreground),
            muted: f(self.muted),
            muted_foreground: f(self.muted_foreground),
            accent: f(self.accent),
            accent_foreground: f(self.accent_foreground),
            destructive: f(self.destructive),
            border: f(self.border),
            input: f(self.input),
            ring: f(self.ring),
            chart: self.chart.map(&f),
            sidebar: f(self.sidebar),
            sidebar_foreground: f(self.sidebar_foreground),
            sidebar_primary: f(self.sidebar_primary),
            sidebar_primary_foreground: f(self.sidebar_primary_foreground),
            sidebar_accent: f(self.sidebar_accent),
            sidebar_accent_foreground: f(self.sidebar_accent_foreground),
            sidebar_border: f(self.sidebar_border),
            sidebar_ring: f(self.sidebar_ring),
            code: f(self.code),
            code_foreground: f(self.code_foreground),
        }
    }
}

impl Palette {
    /// Every token painted: clipped into sRGB channel by channel, as Chrome paints it on an sRGB surface.
    #[must_use]
    pub fn painted(self) -> Palette<Color> {
        self.map(Color::from)
    }

    /// `menuAccent: bold` (shadcn-vue `registry/config.ts:346-355`): the accent tokens take the primary ones.
    /// shadcn-vue also sets `sidebar-accent(-foreground)`; shadcn-ui has those two lines commented out. The verdict
    /// compares against shadcn-vue, so this follows it.
    #[must_use]
    pub const fn bold(self) -> Self {
        Self {
            accent: self.primary,
            accent_foreground: self.primary_foreground,
            sidebar_accent: self.primary,
            sidebar_accent_foreground: self.primary_foreground,
            ..self
        }
    }
}

/// A light and a dark [`Palette`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palettes<C = Oklch> {
    pub light: Palette<C>,
    pub dark: Palette<C>,
}

impl<C: Copy> Palettes<C> {
    /// The palette of `scheme`.
    #[must_use]
    pub const fn get(&self, scheme: super::Scheme) -> &Palette<C> {
        match scheme {
            super::Scheme::Light => &self.light,
            super::Scheme::Dark => &self.dark,
        }
    }

    /// Both palettes with every token passed through `f`.
    #[must_use]
    pub fn map<D>(self, f: impl Fn(C) -> D) -> Palettes<D> {
        Palettes {
            light: self.light.map(&f),
            dark: self.dark.map(&f),
        }
    }
}

impl Palettes {
    /// Both palettes with [`Palette::bold`].
    #[must_use]
    pub const fn bold(self) -> Self {
        Self {
            light: self.light.bold(),
            dark: self.dark.bold(),
        }
    }
}

fn oklch([l, c, h, alpha]: Raw) -> Oklch {
    Oklch::new(l, c, h).with_alpha(alpha)
}

/// One of shadcn's seven base colours (shadcn-vue `registry/base-colors.ts`): the whole palette, light and dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BaseColor {
    #[default]
    Neutral,
    Stone,
    Zinc,
    Mauve,
    Olive,
    Mist,
    Taupe,
}

impl BaseColor {
    pub const ALL: [Self; 7] = [
        Self::Neutral,
        Self::Stone,
        Self::Zinc,
        Self::Mauve,
        Self::Olive,
        Self::Mist,
        Self::Taupe,
    ];

    /// Its name in upstream's registry.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Stone => "stone",
            Self::Zinc => "zinc",
            Self::Mauve => "mauve",
            Self::Olive => "olive",
            Self::Mist => "mist",
            Self::Taupe => "taupe",
        }
    }

    /// Its palettes, light and dark.
    #[must_use]
    pub fn palettes(self) -> Palettes {
        let raw = match self {
            Self::Neutral => data::NEUTRAL,
            Self::Stone => data::STONE,
            Self::Zinc => data::ZINC,
            Self::Mauve => data::MAUVE,
            Self::Olive => data::OLIVE,
            Self::Mist => data::MIST,
            Self::Taupe => data::TAUPE,
        };
        raw.map(oklch)
    }
}

/// One of shadcn's seventeen accent themes: it writes `primary`, `primary-foreground`, `secondary`,
/// `secondary-foreground`, `chart-1…5`, `sidebar-primary` and `sidebar-primary-foreground` over a base colour, in
/// light and dark (rose's dark also writes `sidebar`). Every accent's secondary pair is zinc's, whatever the base, as
/// upstream has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Accent {
    Amber,
    Blue,
    Cyan,
    Emerald,
    Fuchsia,
    Green,
    Indigo,
    Lime,
    Orange,
    Pink,
    Purple,
    Red,
    Rose,
    Sky,
    Teal,
    Violet,
    Yellow,
}

impl Accent {
    pub const ALL: [Self; 17] = [
        Self::Amber,
        Self::Blue,
        Self::Cyan,
        Self::Emerald,
        Self::Fuchsia,
        Self::Green,
        Self::Indigo,
        Self::Lime,
        Self::Orange,
        Self::Pink,
        Self::Purple,
        Self::Red,
        Self::Rose,
        Self::Sky,
        Self::Teal,
        Self::Violet,
        Self::Yellow,
    ];

    /// Its name in upstream's registry.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Amber => "amber",
            Self::Blue => "blue",
            Self::Cyan => "cyan",
            Self::Emerald => "emerald",
            Self::Fuchsia => "fuchsia",
            Self::Green => "green",
            Self::Indigo => "indigo",
            Self::Lime => "lime",
            Self::Orange => "orange",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Red => "red",
            Self::Rose => "rose",
            Self::Sky => "sky",
            Self::Teal => "teal",
            Self::Violet => "violet",
            Self::Yellow => "yellow",
        }
    }

    const fn data(self) -> AccentData {
        match self {
            Self::Amber => data::AMBER,
            Self::Blue => data::BLUE,
            Self::Cyan => data::CYAN,
            Self::Emerald => data::EMERALD,
            Self::Fuchsia => data::FUCHSIA,
            Self::Green => data::GREEN,
            Self::Indigo => data::INDIGO,
            Self::Lime => data::LIME,
            Self::Orange => data::ORANGE,
            Self::Pink => data::PINK,
            Self::Purple => data::PURPLE,
            Self::Red => data::RED,
            Self::Rose => data::ROSE,
            Self::Sky => data::SKY,
            Self::Teal => data::TEAL,
            Self::Violet => data::VIOLET,
            Self::Yellow => data::YELLOW,
        }
    }

    /// `palettes` with this accent written over them (upstream's `buildRegistryTheme`).
    #[must_use]
    pub fn over(self, palettes: Palettes) -> Palettes {
        let d = self.data();
        Palettes {
            light: over(palettes.light, &d.light),
            dark: over(palettes.dark, &d.dark),
        }
    }
}

fn over(p: Palette, o: &data::Overrides) -> Palette {
    Palette {
        primary: oklch(o.primary),
        primary_foreground: oklch(o.primary_foreground),
        secondary: oklch(o.secondary),
        secondary_foreground: oklch(o.secondary_foreground),
        chart: o.chart.map(oklch),
        sidebar_primary: oklch(o.sidebar_primary),
        sidebar_primary_foreground: oklch(o.sidebar_primary_foreground),
        sidebar: o.sidebar.map_or(p.sidebar, oklch),
        ..p
    }
}
