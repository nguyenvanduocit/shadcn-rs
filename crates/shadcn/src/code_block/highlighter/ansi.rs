//! Terminal output (`ansi`) as Shiki tokenizes it (`tokenizeAnsiWithTheme`,
//! `@shikijs/core`): SGR escapes pick colours from the theme's
//! `terminal.ansi*` palette, and dim text takes its colour at half alpha.
//! irosashi's own `ansi` path uses VS Code's fixed palette and drops dim, so
//! the code block reads the escapes itself; the theme colours still come from
//! irosashi.

use irosashi::Highlighter;

use super::{HighlightedToken, TokenColors, parse_hex};

/// Shiki's `namedColors`, in SGR order (30–37, then 90–97).
const NAMES: [&str; 16] = [
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Magenta",
    "Cyan",
    "White",
    "BrightBlack",
    "BrightRed",
    "BrightGreen",
    "BrightYellow",
    "BrightBlue",
    "BrightMagenta",
    "BrightCyan",
    "BrightWhite",
];

/// Shiki's `defaultAnsiColors`, for a theme that names none.
const DEFAULTS: [&str; 16] = [
    "#000000", "#cd3131", "#0DBC79", "#E5E510", "#2472C8", "#BC3FBC", "#11A8CD", "#E5E5E5", "#666666", "#F14C4C",
    "#23D18B", "#F5F543", "#3B8EEA", "#D670D6", "#29B8DB", "#FFFFFF",
];

/// The six levels of xterm's 256-colour cube.
const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnsiColor {
    /// One of the 16 named colours.
    Named(usize),
    Rgb([u8; 3]),
    /// An index into the 256-colour table.
    Table(u16),
}

/// The SGR state in force: what of it changes a token's colour.
#[derive(Clone, Copy, Debug, Default)]
struct Sgr {
    foreground: Option<AnsiColor>,
    background: Option<AnsiColor>,
    dim: bool,
    reverse: bool,
}

impl Sgr {
    /// Apply one escape's parameters as Shiki's `parseSequence` reads them:
    /// every reset first, then every set.
    fn apply(&mut self, params: &str) {
        let mut sets = Vec::new();
        let mut parts = params.split(';');
        while let Some(part) = parts.next() {
            // `if (!code) continue`: an empty parameter does nothing.
            let Ok(code) = part.parse::<u16>() else {
                continue;
            };
            match code {
                0 => *self = Self::default(),
                2 | 7 => sets.push(Set::Decoration(code)),
                22 => self.dim = false,
                27 => self.reverse = false,
                30..=37 => sets.push(Set::Foreground(AnsiColor::Named(usize::from(code.saturating_sub(30))))),
                38 => sets.extend(color(&mut parts).map(Set::Foreground)),
                39 => self.foreground = None,
                40..=47 => sets.push(Set::Background(AnsiColor::Named(usize::from(code.saturating_sub(40))))),
                48 => sets.extend(color(&mut parts).map(Set::Background)),
                49 => self.background = None,
                90..=97 => sets.push(Set::Foreground(AnsiColor::Named(usize::from(code.saturating_sub(82))))),
                100..=107 => sets.push(Set::Background(AnsiColor::Named(usize::from(code.saturating_sub(92))))),
                _ => {}
            }
        }
        for set in sets {
            match set {
                Set::Foreground(c) => self.foreground = Some(c),
                Set::Background(c) => self.background = Some(c),
                Set::Decoration(2) => self.dim = true,
                Set::Decoration(_) => self.reverse = true,
            }
        }
    }
}

enum Set {
    Foreground(AnsiColor),
    Background(AnsiColor),
    Decoration(u16),
}

/// `38;5;n` or `38;2;r;g;b` (Shiki's `parseColor`).
fn color<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<AnsiColor> {
    match parts.next()? {
        "2" => {
            // Shiki takes the next three parameters, valid or not.
            let [r, g, b]: [Option<i64>; 3] = std::array::from_fn(|_| parts.next().and_then(|p| p.parse().ok()));
            let channel = |c: i64| u8::try_from(c.clamp(0, 255)).unwrap_or(u8::MAX);
            Some(AnsiColor::Rgb([channel(r?), channel(g?), channel(b?)]))
        }
        "5" => parts.next().and_then(|p| p.parse().ok()).map(AnsiColor::Table),
        _ => None,
    }
}

/// One theme's terminal palette and its default colours.
struct Palette {
    named: [String; 16],
    foreground: String,
    background: String,
}

impl Palette {
    fn of(highlighter: &Highlighter, theme: &str) -> Option<Self> {
        let colors = highlighter.theme_colors(theme).ok()?;
        let named = std::array::from_fn(|i| {
            NAMES
                .get(i)
                .and_then(|name| colors.colors.get(&format!("terminal.ansi{name}")))
                .map_or_else(|| DEFAULTS.get(i).copied().unwrap_or_default().to_owned(), Clone::clone)
        });
        Some(Self {
            named,
            foreground: colors.foreground,
            background: colors.background,
        })
    }

    fn value(&self, color: AnsiColor) -> Option<String> {
        match color {
            AnsiColor::Named(i) => self.named.get(i).cloned(),
            AnsiColor::Rgb([r, g, b]) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
            AnsiColor::Table(i) => match i {
                0..=15 => self.named.get(usize::from(i)).cloned(),
                16..=231 => {
                    let cube = i.saturating_sub(16);
                    let level = |n: u16| LEVELS.get(usize::from(n % 6)).copied().unwrap_or_default();
                    let (r, g, b) = (level(cube / 36), level(cube / 6), level(cube));
                    Some(format!("#{r:02x}{g:02x}{b:02x}"))
                }
                232..=255 => {
                    let v = u8::try_from(i.saturating_sub(232).saturating_mul(10).saturating_add(8)).unwrap_or(u8::MAX);
                    Some(format!("#{v:02x}{v:02x}{v:02x}"))
                }
                _ => None,
            },
        }
    }

    /// The colour a token takes under `sgr`; `None` for a table index past
    /// 255, which Shiki leaves uncoloured.
    fn color(&self, sgr: Sgr) -> Option<String> {
        let (set, default) = if sgr.reverse {
            (sgr.background, &self.background)
        } else {
            (sgr.foreground, &self.foreground)
        };
        let base = match set {
            Some(c) => self.value(c)?,
            None => default.clone(),
        };
        Some(if sgr.dim { dim(&base) } else { base })
    }
}

/// Shiki's `dimColor`: a hex colour at half its alpha.
fn dim(hex: &str) -> String {
    let Some(digits) = hex.strip_prefix('#') else {
        return hex.to_owned();
    };
    let half = |alpha: &str| {
        u8::from_str_radix(alpha, 16)
            .ok()
            .map(|a| format!("{:02x}", a.div_ceil(2)))
    };
    let chars: Vec<char> = digits.chars().collect();
    match chars.as_slice() {
        [_, _, _, _, _, _, _, _] => match (digits.get(..6), digits.get(6..).and_then(half)) {
            (Some(rgb), Some(alpha)) => format!("#{rgb}{alpha}"),
            _ => hex.to_owned(),
        },
        [_, _, _, _, _, _] => format!("#{digits}80"),
        [r, g, b, a] => match half(&format!("{a}{a}")) {
            Some(alpha) => format!("#{r}{r}{g}{g}{b}{b}{alpha}"),
            None => hex.to_owned(),
        },
        [r, g, b] => format!("#{r}{r}{g}{g}{b}{b}80"),
        _ => hex.to_owned(),
    }
}

/// `code`'s lines as tokens coloured in GitHub's light and dark terminal
/// palettes; the SGR state carries from one line to the next.
pub(super) fn tokenize(highlighter: &Highlighter, code: &str) -> Option<Vec<Vec<HighlightedToken>>> {
    let light = Palette::of(highlighter, "github-light")?;
    let dark = Palette::of(highlighter, "github-dark")?;
    let mut sgr = Sgr::default();
    Some(
        code.split('\n')
            .map(|line| {
                let line = line.strip_suffix('\r').unwrap_or(line);
                let mut tokens = Vec::new();
                let mut rest = line;
                loop {
                    // Shiki's `findSequence`: `ESC [` up to the next `m`; an
                    // escape that is not one leaves the rest of the line as text.
                    let found = rest.find('\x1B').and_then(|at| {
                        let after = rest.get(at.saturating_add(1)..)?;
                        let params = after.strip_prefix('[')?;
                        let end = params.find('m')?;
                        Some((at, params.get(..end)?, params.get(end.saturating_add(1)..)?))
                    });
                    let text = found.map_or(rest, |(at, _, _)| rest.get(..at).unwrap_or_default());
                    if !text.is_empty() {
                        let colors = light
                            .color(sgr)
                            .and_then(|c| parse_hex(&c))
                            .zip(dark.color(sgr).and_then(|c| parse_hex(&c)))
                            .map(|(light, dark)| TokenColors { light, dark });
                        tokens.push(HighlightedToken {
                            content: text.to_owned(),
                            colors,
                        });
                    }
                    let Some((_, params, after)) = found else {
                        break;
                    };
                    sgr.apply(params);
                    rest = after;
                }
                tokens
            })
            .collect(),
    )
}
