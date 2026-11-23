//! A theme written as the choices of shadcn's theme picker: `baseColor=zinc,theme=blue,radius=large`. The keys and
//! values are upstream's (`designSystemConfigSchema`, shadcn-vue `apps/v4/registry/config.ts`), so the same text
//! names the same theme to the docs app (`--theme`) and to the verdict's browser, which builds upstream's CSS variables
//! from it.

use std::fmt;
use std::str::FromStr;

use super::{Accent, BaseColor, Font, Menu, MenuAccent, Radius, Theme};
use crate::icons::IconSet;

/// The picker's choices. [`ThemeSpec::default`] is upstream's `DEFAULT_CONFIG`; [`ThemeSpec::theme`] builds the
/// [`Theme`] they name. Written back ([`fmt::Display`]) with every key, in [`ThemeSpec::KEYS`] order, so two specs
/// naming the same choices print the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeSpec {
    /// `baseColor`.
    pub base: BaseColor,
    /// `theme`: an accent written over the base colour, or `None` for the base colour's own (upstream names it
    /// after the base colour).
    pub accent: Option<Accent>,
    /// `radius`.
    pub radius: Radius,
    /// `font`.
    pub font: Font,
    /// `fontHeading`: `None` is `inherit`.
    pub heading: Option<Font>,
    /// `menuAccent` and `menuColor`.
    pub menu: Menu,
    /// `iconLibrary`.
    pub icons: IconSet,
}

impl Default for ThemeSpec {
    fn default() -> Self {
        Self {
            base: BaseColor::Neutral,
            accent: None,
            radius: Radius::Default,
            font: Font::Inter,
            heading: None,
            menu: Menu::default(),
            icons: IconSet::DEFAULT,
        }
    }
}

/// Why a spec names no theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecError {
    /// A `key=value` pair without the `=`.
    NotAPair(String),
    UnknownKey(String),
    /// A key given twice.
    Repeated(&'static str),
    /// A value its key has no choice of; `choices` lists those it has.
    UnknownValue {
        key: &'static str,
        value: String,
        choices: Vec<&'static str>,
    },
    /// `theme` names another base colour than `baseColor` (upstream's schema refuses it too).
    ThemeOfOtherBase {
        base: BaseColor,
        theme: BaseColor,
    },
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPair(p) => write!(f, "{p:?} is not key=value"),
            Self::UnknownKey(k) => write!(f, "unknown key {k:?}; keys: {}", ThemeSpec::KEYS.join(", ")),
            Self::Repeated(k) => write!(f, "{k} is given twice"),
            Self::UnknownValue { key, value, choices } => {
                write!(f, "{key}={value}: no such choice; choices: {}", choices.join(", "))
            }
            Self::ThemeOfOtherBase { base, theme } => write!(
                f,
                "theme={} is base colour {}'s own theme, not available for baseColor={} (pick an accent or {})",
                theme.name(),
                theme.name(),
                base.name(),
                base.name()
            ),
        }
    }
}

impl std::error::Error for SpecError {}

const RADII: [(Radius, &str); 5] = [
    (Radius::Default, "default"),
    (Radius::None, "none"),
    (Radius::Small, "small"),
    (Radius::Medium, "medium"),
    (Radius::Large, "large"),
];

const MENU_ACCENTS: [(MenuAccent, &str); 2] = [(MenuAccent::Subtle, "subtle"), (MenuAccent::Bold, "bold")];

/// `menuColor`'s values, as `(inverted, translucent)`.
const MENU_COLORS: [((bool, bool), &str); 4] = [
    ((false, false), "default"),
    ((false, true), "default-translucent"),
    ((true, false), "inverted"),
    ((true, true), "inverted-translucent"),
];

const INHERIT: &str = "inherit";

fn name_of<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> &'static str {
    table.iter().find(|(v, _)| *v == value).map_or("", |(_, n)| n)
}

fn choose<T: Copy>(
    key: &'static str,
    value: &str,
    choices: impl Iterator<Item = (T, &'static str)> + Clone,
) -> Result<T, SpecError> {
    choices
        .clone()
        .find(|(_, n)| *n == value)
        .map(|(v, _)| v)
        .ok_or_else(|| SpecError::UnknownValue {
            key,
            value: value.to_owned(),
            choices: choices.map(|(_, n)| n).collect(),
        })
}

impl ThemeSpec {
    /// The keys, as upstream names them.
    pub const KEYS: [&'static str; 8] = [
        "baseColor",
        "theme",
        "radius",
        "font",
        "fontHeading",
        "menuAccent",
        "menuColor",
        "iconLibrary",
    ];

    /// Parses comma-separated `key=value` pairs (spaces around either are ignored); a key not given keeps its
    /// default. The empty spec is the default.
    pub fn parse(spec: &str) -> Result<Self, SpecError> {
        let mut s = Self::default();
        let mut seen: Vec<&'static str> = Vec::new();
        let mut theme: Option<&str> = None;
        for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .map(|(k, v)| (k.trim(), v.trim()))
                .ok_or_else(|| SpecError::NotAPair(pair.to_owned()))?;
            let key = Self::KEYS
                .iter()
                .copied()
                .find(|k| *k == key)
                .ok_or_else(|| SpecError::UnknownKey(key.to_owned()))?;
            if seen.contains(&key) {
                return Err(SpecError::Repeated(key));
            }
            seen.push(key);
            match key {
                "baseColor" => s.base = choose(key, value, BaseColor::ALL.iter().map(|b| (*b, b.name())))?,
                // Read once the base colour is known, whatever the order.
                "theme" => theme = Some(value),
                "radius" => s.radius = choose(key, value, RADII.iter().copied())?,
                "font" => s.font = choose(key, value, Font::ALL.iter().map(|f| (*f, f.name())))?,
                "fontHeading" => {
                    let fonts = Font::ALL.iter().map(|f| (Some(*f), f.name()));
                    s.heading = choose(key, value, std::iter::once((None, INHERIT)).chain(fonts))?;
                }
                "menuAccent" => s.menu.accent = choose(key, value, MENU_ACCENTS.iter().copied())?,
                "menuColor" => {
                    (s.menu.inverted, s.menu.translucent) = choose(key, value, MENU_COLORS.iter().copied())?;
                }
                _ => s.icons = choose(key, value, IconSet::ALL.iter().map(|i| (*i, i.name())))?,
            }
        }
        if let Some(value) = theme {
            s.accent = match BaseColor::ALL.iter().find(|b| b.name() == value) {
                Some(&b) if b == s.base => None,
                Some(&b) => return Err(SpecError::ThemeOfOtherBase { base: s.base, theme: b }),
                // The base colour's own name was matched above; it leads the choices an error lists.
                None => choose(
                    "theme",
                    value,
                    std::iter::once((None, s.base.name())).chain(Accent::ALL.iter().map(|a| (Some(*a), a.name()))),
                )?,
            };
        }
        Ok(s)
    }

    /// The value of `key` ([`ThemeSpec::KEYS`]), as upstream writes it; `None` for another key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&'static str> {
        Some(match key {
            "baseColor" => self.base.name(),
            "theme" => self.accent.map_or(self.base.name(), Accent::name),
            "radius" => name_of(&RADII, self.radius),
            "font" => self.font.name(),
            "fontHeading" => self.heading.map_or(INHERIT, Font::name),
            "menuAccent" => name_of(&MENU_ACCENTS, self.menu.accent),
            "menuColor" => name_of(&MENU_COLORS, (self.menu.inverted, self.menu.translucent)),
            "iconLibrary" => self.icons.name(),
            _ => return None,
        })
    }

    /// These choices as upstream's picker keeps them in the style named `style` (shadcn `apps/v4`
    /// `design-system-provider.tsx`, the effect that rewrites the search params): lyra and sera are square, so
    /// their radius is `none` whatever was picked; rhea has no `large` radius and takes the default. Every other
    /// style keeps the choices. Both sides of the verdict read the spec after this, so they judge one radius.
    #[must_use]
    pub fn in_style(mut self, style: &str) -> Self {
        match (style, self.radius) {
            ("lyra" | "sera", _) => self.radius = Radius::None,
            ("rhea", Radius::Large) => self.radius = Radius::Default,
            _ => {}
        }
        self
    }

    /// The theme these choices name, light.
    #[must_use]
    pub fn theme(&self) -> Theme {
        let mut t = Theme::new(self.base)
            .radius(self.radius)
            .font(self.font)
            .menu(self.menu)
            .icons(self.icons);
        if let Some(accent) = self.accent {
            t = t.accent(accent);
        }
        if let Some(heading) = self.heading {
            t = t.heading(heading);
        }
        t
    }
}

impl FromStr for ThemeSpec {
    type Err = SpecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for ThemeSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, key) in Self::KEYS.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            write!(f, "{sep}{key}={}", self.get(key).unwrap_or_default())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Heading;

    #[test]
    fn empty_is_upstream_default() {
        let s = ThemeSpec::parse("").unwrap();
        assert_eq!(s, ThemeSpec::default());
        assert_eq!(s.theme(), Theme::default());
        assert_eq!(
            s.to_string(),
            "baseColor=neutral,theme=neutral,radius=default,font=inter,fontHeading=inherit,menuAccent=subtle,\
             menuColor=default,iconLibrary=lucide"
        );
    }

    #[test]
    fn every_choice_reads_back() {
        let spec = "baseColor=zinc,theme=blue,radius=large,font=geist-mono,fontHeading=playfair-display,\
                    menuAccent=bold,menuColor=inverted-translucent,iconLibrary=lucide";
        let s = ThemeSpec::parse(spec).unwrap();
        assert_eq!(s.to_string(), spec);
        assert_eq!(ThemeSpec::parse(&s.to_string()).unwrap(), s);
        let t = s.theme();
        let want = Theme::new(BaseColor::Zinc)
            .accent(Accent::Blue)
            .radius(Radius::Large)
            .font(Font::GeistMono)
            .heading(Font::PlayfairDisplay)
            .menu(Menu {
                accent: MenuAccent::Bold,
                inverted: true,
                translucent: true,
            });
        assert_eq!(t, want);
        assert_eq!(t.radius, 14.0);
        assert_eq!(t.fonts.heading, Heading::Family(Font::PlayfairDisplay.family()));
    }

    #[test]
    fn order_and_spaces_do_not_matter() {
        let a = ThemeSpec::parse("theme=rose, baseColor = stone ,").unwrap();
        let b = ThemeSpec::parse("baseColor=stone,theme=rose").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.accent, Some(Accent::Rose));
    }

    #[test]
    fn theme_may_name_its_own_base() {
        let s = ThemeSpec::parse("baseColor=taupe,theme=taupe").unwrap();
        assert_eq!((s.base, s.accent), (BaseColor::Taupe, None));
        assert_eq!(
            ThemeSpec::parse("baseColor=taupe,theme=zinc"),
            Err(SpecError::ThemeOfOtherBase {
                base: BaseColor::Taupe,
                theme: BaseColor::Zinc
            })
        );
    }

    #[test]
    fn mistakes_name_what_is_wrong() {
        let err = |s: &str| ThemeSpec::parse(s).unwrap_err().to_string();
        assert!(err("baseColor").contains("is not key=value"));
        assert!(err("colour=red").contains("unknown key \"colour\"; keys: baseColor, theme"));
        assert!(err("radius=huge").contains("radius=huge: no such choice; choices: default, none, small"));
        assert!(err("theme=teal,theme=red").contains("theme is given twice"));
        assert!(err("theme=crimson").contains("choices: neutral, amber"));
        assert!(err("iconLibrary=feather").contains("lucide"));
    }

    #[test]
    fn a_style_keeps_the_radii_upstream_lets_it_have() {
        let radius = |style: &str, r: &str| {
            let s = ThemeSpec::parse(&format!("radius={r},theme=blue"))
                .unwrap()
                .in_style(style);
            assert_eq!(s.accent, Some(Accent::Blue), "only the radius changes");
            s.get("radius").unwrap()
        };
        for r in ["default", "none", "small", "medium", "large"] {
            assert_eq!(radius("lyra", r), "none");
            assert_eq!(radius("sera", r), "none");
            assert_eq!(radius("vega", r), r);
            assert_eq!(radius("rhea", r), if r == "large" { "default" } else { r });
        }
    }

    #[test]
    fn get_answers_every_key_only() {
        let s = ThemeSpec::default();
        for key in ThemeSpec::KEYS {
            assert!(s.get(key).is_some_and(|v| !v.is_empty()), "{key}");
        }
        assert_eq!(s.get("style"), None);
    }
}
