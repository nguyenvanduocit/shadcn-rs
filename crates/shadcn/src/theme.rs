//! shadcn's design tokens, the neutral base colour in light and dark, as
//! `registry/themes.ts` defines them. The one place a component reads a colour from.

use gui::{Color, Signal, provide, use_context};

/// Every colour token shadcn components use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub background: Color,
    pub foreground: Color,
    pub card: Color,
    pub card_foreground: Color,
    pub popover: Color,
    pub popover_foreground: Color,
    pub primary: Color,
    pub primary_foreground: Color,
    pub secondary: Color,
    pub secondary_foreground: Color,
    pub muted: Color,
    pub muted_foreground: Color,
    pub accent: Color,
    pub accent_foreground: Color,
    pub destructive: Color,
    pub border: Color,
    pub input: Color,
    pub ring: Color,
    pub chart: [Color; 5],
    pub sidebar: Color,
    pub sidebar_foreground: Color,
    pub sidebar_primary: Color,
    pub sidebar_primary_foreground: Color,
    pub sidebar_accent: Color,
    pub sidebar_accent_foreground: Color,
    pub sidebar_border: Color,
    pub sidebar_ring: Color,
    /// shadcn-vue's site adds these for code its demos show (`bg-code
    /// text-code-foreground`): `--code` is its `--surface`.
    pub code: Color,
    pub code_foreground: Color,
}

fn grey(l: f32) -> Color {
    Color::oklch(l, 0.0, 0.0)
}

impl Theme {
    #[must_use]
    pub fn light() -> Self {
        Self {
            dark: false,
            background: grey(1.0),
            foreground: grey(0.145),
            card: grey(1.0),
            card_foreground: grey(0.145),
            popover: grey(1.0),
            popover_foreground: grey(0.145),
            primary: grey(0.205),
            primary_foreground: grey(0.985),
            secondary: grey(0.97),
            secondary_foreground: grey(0.205),
            muted: grey(0.97),
            muted_foreground: grey(0.556),
            accent: grey(0.97),
            accent_foreground: grey(0.205),
            destructive: Color::oklch(0.577, 0.245, 27.325),
            border: grey(0.922),
            input: grey(0.922),
            ring: grey(0.708),
            chart: [grey(0.87), grey(0.556), grey(0.439), grey(0.371), grey(0.269)],
            sidebar: grey(0.985),
            sidebar_foreground: grey(0.145),
            sidebar_primary: grey(0.205),
            sidebar_primary_foreground: grey(0.985),
            sidebar_accent: grey(0.97),
            sidebar_accent_foreground: grey(0.205),
            sidebar_border: grey(0.922),
            sidebar_ring: grey(0.708),
            code: grey(0.98),
            code_foreground: grey(0.145),
        }
    }

    #[must_use]
    pub fn dark() -> Self {
        Self {
            dark: true,
            background: grey(0.145),
            foreground: grey(0.985),
            card: grey(0.205),
            card_foreground: grey(0.985),
            popover: grey(0.205),
            popover_foreground: grey(0.985),
            primary: grey(0.922),
            primary_foreground: grey(0.205),
            secondary: grey(0.269),
            secondary_foreground: grey(0.985),
            muted: grey(0.269),
            muted_foreground: grey(0.708),
            accent: grey(0.269),
            accent_foreground: grey(0.985),
            destructive: Color::oklch(0.704, 0.191, 22.216),
            border: grey(1.0).opacity(0.1),
            input: grey(1.0).opacity(0.15),
            ring: grey(0.556),
            chart: [grey(0.87), grey(0.556), grey(0.439), grey(0.371), grey(0.269)],
            sidebar: grey(0.205),
            sidebar_foreground: grey(0.985),
            sidebar_primary: Color::oklch(0.488, 0.243, 264.376),
            sidebar_primary_foreground: grey(0.985),
            sidebar_accent: grey(0.269),
            sidebar_accent_foreground: grey(0.985),
            sidebar_border: grey(1.0).opacity(0.1),
            sidebar_ring: grey(0.556),
            code: grey(0.2),
            code_foreground: grey(0.708),
        }
    }
}

/// Make `theme` the one components read; switching it re-renders them.
pub fn provide_theme(theme: Signal<Theme>) {
    provide(theme);
}

/// The current theme (reactive); light when none was provided.
#[must_use]
pub fn theme() -> Theme {
    use_context::<Signal<Theme>>().map_or_else(Theme::light, |t| t.get())
}
