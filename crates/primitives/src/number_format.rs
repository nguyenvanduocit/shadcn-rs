//! How a number field writes and reads its number in a locale (reka's
//! `NumberFormatter` and `NumberParser` from `@internationalized/number`,
//! over `Intl.NumberFormat`; research §10 "NumberField").
//!
//! The locale's CLDR data comes from ICU4X: separators, digits of its
//! numbering system (`ar-EG` writes `١٢٣`, `-u-nu-` picks another), the
//! sizes of its groups (`en-IN` writes `12,34,567`) and its signs
//! (`icu_decimal`); the patterns of a percentage, a currency shown by its
//! code (with its own decimals, `JPY` none) and the accounting form
//! (`icu_experimental`'s percent and currency formatters); the name of a
//! unit, by plural form (`icu_experimental`'s unit names, `icu_plurals`).
//!
//! - **Writing** ([`NumberFormat::format`]): the value rounded half away
//!   from zero to at most `max_fraction` decimals, then written with at
//!   least `min_fraction` (a currency keeps its own); a percentage is the
//!   value times 100. No minus on zero.
//! - **Reading** ([`NumberFormat::parse`]): what this format writes, or
//!   what is plainly typed. As `NumberParser` does, the locale's separators
//!   are learned by writing sample numbers: a group separator is dropped,
//!   the decimal separator is the point (`de-DE`: `1.234` is 1234, `1,5`
//!   is 1.5); the literal text around the number (a code, `%`, a unit's
//!   name in any plural form, spaces) is dropped; a minus, an accounting
//!   parenthesis or the locale's own minus makes it negative. Digits of
//!   every script are read, as are the Han numerals `〇一二三四五六七八九`
//!   (reka's `hanidec`), in any locale (`en-US` reads `٢٥%` as 0.25).
//! - **In progress** ([`NumberFormat::partial`]): what may stand in the
//!   input while typing: a sign (a minus only where negative values are
//!   allowed), digits, group separators, one decimal separator when the
//!   format has decimals, and the format's literals.
//!
//! The formatters a format needs are built once per thread and format, and
//! shared.

#[expect(
    clippy::disallowed_types,
    reason = "a per-thread cache of resolved formats, keyed by their options: no component's state"
)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fixed_decimal::{Decimal, FloatPrecision, Sign};
use icu_decimal::DecimalFormatter;
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_experimental::dimension::currency::formatter::CurrencyFormatter;
use icu_experimental::dimension::currency::options::{CurrencyFormatterOptions, CurrencyUsage};
use icu_experimental::dimension::percent::formatter::PercentFormatter;
use icu_experimental::dimension::percent::options::{Display, PercentFormatterOptions};
use icu_experimental::dimension::provider::units::display_names::UnitsDisplayNames;
use icu_locale_core::Locale;
use icu_locale_core::preferences::extensions::unicode::keywords::CurrencyType;
use icu_plurals::PluralRules;
use writeable::Writeable;

/// How the value is written (`format-options`, `Intl.NumberFormatOptions`,
/// and the locale): plain en-US by default, grouped, at most three decimals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    pub locale: Locale,
    pub style: NumberStyle,
    pub sign: SignDisplay,
    /// Decimals always written (`minimumFractionDigits`); a currency's own
    /// count rules it.
    pub min_fraction: usize,
    /// Decimals at most (`maximumFractionDigits`); a currency's own count
    /// rules it.
    pub max_fraction: usize,
}

/// `style`: a plain number, a percentage, an amount of a currency, or a
/// measure of a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberStyle {
    Decimal,
    /// `percent`: the value times 100, in the locale's percent pattern.
    Percent,
    /// `currency` shown by its ISO `code` (`currencyDisplay: 'code'`);
    /// `accounting` writes a negative amount as the locale's accounting
    /// pattern does (`currencySign: 'accounting'`; en-US: parentheses).
    Currency {
        code: &'static str,
        accounting: bool,
    },
    /// `unit`: a CLDR unit (`"inch"`, `"kilometer"`, `"minute"`,
    /// `"liter"`, `"kilogram"`, `"square-meter"`…), its name as wide as
    /// `width` says (`unitDisplay`).
    Unit {
        unit: &'static str,
        width: UnitWidth,
    },
}

impl gui::engine::FreshValue for NumberStyle {}

/// `unitDisplay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum UnitWidth {
    /// `5 inches`.
    Long,
    /// `5 in`.
    #[default]
    Short,
    /// `5″`.
    Narrow,
}

/// `signDisplay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SignDisplay {
    /// A minus on negatives only.
    #[default]
    Auto,
    /// A plus on positives and a minus on negatives; zero bare.
    ExceptZero,
}

impl Default for NumberFormat {
    fn default() -> Self {
        Self {
            locale: icu_locale_core::locale!("en-US"),
            style: NumberStyle::Decimal,
            sign: SignDisplay::Auto,
            min_fraction: 0,
            max_fraction: 3,
        }
    }
}

/// The Han numerals reka reads as digits (`hanidec`), 0 to 9.
const HAN: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// The zero of each run of ten decimal digits Unicode has (`Nd`) outside
/// ASCII, in the Basic Multilingual Plane.
const ZEROS: [u32; 36] = [
    0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6, 0x0D66, 0x0DE6, 0x0E50,
    0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50,
    0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
];

/// The digit `c` stands for, in any script.
#[must_use]
pub fn digit(c: char) -> Option<u32> {
    if let Some(d) = c.to_digit(10) {
        return Some(d);
    }
    if let Some(d) = HAN.iter().position(|h| *h == c) {
        return u32::try_from(d).ok();
    }
    let code = u32::from(c);
    ZEROS.iter().find_map(|z| code.checked_sub(*z).filter(|d| *d < 10))
}

/// Marks that steer bidirectional text and carry nothing else
/// (`ar` writes `؜-`), dropped before reading.
fn is_bidi_mark(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// What reads a typed number in one format: what the locale writes
/// between and around the digits.
#[derive(Debug)]
struct Symbols {
    decimal: String,
    group: String,
    minus: String,
    plus: String,
    /// The text around the number (a code, `%`, a unit's names), longest
    /// first so a long name is dropped before a short one inside it.
    literals: Vec<String>,
}

/// What writes the number around its digits.
enum Body {
    Decimal,
    Percent {
        plain: PercentFormatter<DecimalFormatter>,
        signed: PercentFormatter<DecimalFormatter>,
    },
    Currency(CurrencyFormatter<DecimalFormatter>),
    Unit(Box<UnitNames>),
}

/// A unit's name by plural form, and the rules that pick one.
struct UnitNames {
    names: icu_provider::DataPayload<icu_provider::marker::ErasedMarker<UnitsDisplayNames<'static>>>,
    rules: PluralRules,
}

/// One format, resolved: its formatters and its reader's symbols.
struct Resolved {
    decimal: DecimalFormatter,
    body: Body,
    symbols: Symbols,
}

type Key = (String, NumberStyle);

gui::engine_local! {
    /// Resolved formats, by their options: a cache, so a fork reads none
    /// and warms none (it resolves its own).
    #[expect(clippy::disallowed_types, reason = "a per-thread cache of resolved formats, keyed by their options: no component's state")]
    [Fresh] static RESOLVED: RefCell<HashMap<Key, Rc<Resolved>>> = RefCell::new(HashMap::new());
}

/// The unit's names in `locale`, `width` wide: the first of the unit
/// categories ICU4X keeps that has it.
fn unit_names(locale: &Locale, unit: &str, width: UnitWidth) -> Option<UnitNames> {
    use icu_experimental::dimension::provider::units::categorized_display_names as names;
    use icu_provider::prelude::*;
    let width = match width {
        UnitWidth::Long => "long",
        UnitWidth::Short => "short",
        UnitWidth::Narrow => "narrow",
    };
    let attribute = format!("{width}-{unit}");
    let attribute = DataMarkerAttributes::try_from_str(&attribute).ok()?;
    let data = DataLocale::from(locale);
    let request = || DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attribute, &data),
        ..Default::default()
    };
    let baked = icu_experimental::provider::Baked;
    macro_rules! first {
        ($($m:ty),* $(,)?) => {
            None $(.or_else(|| {
                DataProvider::<$m>::load(&baked, request()).ok().map(|r| r.payload.cast())
            }))*
        };
    }
    let names: Option<DataPayload<icu_provider::marker::ErasedMarker<UnitsDisplayNames<'static>>>> = first!(
        names::UnitsNamesLengthCoreV1,
        names::UnitsNamesLengthExtendedV1,
        names::UnitsNamesLengthOutlierV1,
        names::UnitsNamesDurationCoreV1,
        names::UnitsNamesDurationExtendedV1,
        names::UnitsNamesDurationOutlierV1,
        names::UnitsNamesMassCoreV1,
        names::UnitsNamesMassExtendedV1,
        names::UnitsNamesMassOutlierV1,
        names::UnitsNamesVolumeCoreV1,
        names::UnitsNamesVolumeExtendedV1,
        names::UnitsNamesVolumeOutlierV1,
        names::UnitsNamesAreaCoreV1,
        names::UnitsNamesAreaExtendedV1,
        names::UnitsNamesAreaOutlierV1,
    );
    let rules = PluralRules::try_new_cardinal(locale.into()).ok()?;
    Some(UnitNames { names: names?, rules })
}

impl Resolved {
    fn new(locale: &Locale, style: NumberStyle) -> Self {
        let decimal = decimal_formatter(locale, GroupingStrategy::Auto);
        let body = match style {
            NumberStyle::Decimal => Body::Decimal,
            NumberStyle::Percent => {
                let percent =
                    |display| PercentFormatter::try_new(locale.into(), PercentFormatterOptions::from(display)).ok();
                match (percent(Display::Standard), percent(Display::ExplicitSign)) {
                    (Some(plain), Some(signed)) => Body::Percent { plain, signed },
                    _ => Body::Decimal,
                }
            }
            NumberStyle::Currency { code, accounting } => {
                let usage = if accounting {
                    CurrencyUsage::Accounting
                } else {
                    CurrencyUsage::Standard
                };
                let currency = CurrencyType::try_from_str(&code.to_ascii_lowercase()).ok();
                let formatter = currency.and_then(|c| {
                    CurrencyFormatter::try_new_code(locale.into(), c, CurrencyFormatterOptions::from(usage)).ok()
                });
                match formatter {
                    Some(f) => Body::Currency(f),
                    None => Body::Decimal,
                }
            }
            NumberStyle::Unit { unit, width } => match unit_names(locale, unit, width) {
                Some(names) => Body::Unit(Box::new(names)),
                None => Body::Decimal,
            },
        };
        let mut resolved = Self {
            decimal,
            body,
            symbols: Symbols {
                decimal: String::new(),
                group: String::new(),
                minus: String::new(),
                plus: String::new(),
                literals: Vec::new(),
            },
        };
        resolved.symbols = resolved.learn(locale);
        resolved
    }

    /// Learn what the locale writes between and around the digits by
    /// writing samples, as `NumberParser` does.
    fn learn(&self, locale: &Locale) -> Symbols {
        let always = decimal_formatter(locale, GroupingStrategy::Always);
        let plain = |s: &str| -> String {
            let d = Decimal::try_from_str(s).unwrap_or_default();
            always
                .format(&d)
                .write_to_string()
                .chars()
                .filter(|c| !is_bidi_mark(*c))
                .collect()
        };
        // Between digits: `1000.5` is 1, group, 000, decimal, 5.
        let digits_of = |t: &str| t.chars().filter(|c| digit(*c).is_none()).collect::<String>();
        let sample = plain("1000.5");
        let mut runs = Vec::new();
        let mut run = String::new();
        for c in sample.chars() {
            if digit(c).is_some() {
                if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            } else {
                run.push(c);
            }
        }
        let (group, decimal) = match runs.as_slice() {
            [g, d] => (g.clone(), d.clone()),
            [d] => (String::new(), d.clone()),
            _ => (",".to_owned(), ".".to_owned()),
        };
        let minus = digits_of(&plain("-1"));
        let plus = digits_of(&plain("+1"));
        // Around the number: what the format writes that is not the plain number.
        let mut literals: Vec<String> = Vec::new();
        for s in ["1", "2", "0.5", "5", "1000", "-1", "-2", "+1"] {
            let Ok(mut d) = Decimal::try_from_str(s) else { continue };
            let whole: String = self.body(&mut d).chars().filter(|c| !is_bidi_mark(*c)).collect();
            // The number runs from its first digit to its last; the rest is literal.
            let first = whole.char_indices().find(|(_, c)| digit(*c).is_some()).map(|(i, _)| i);
            let last = whole
                .char_indices()
                .rev()
                .find(|(_, c)| digit(*c).is_some())
                .map(|(i, c)| i.saturating_add(c.len_utf8()));
            let (Some(first), Some(last)) = (first, last) else {
                continue;
            };
            for piece in [whole.get(..first), whole.get(last..)].into_iter().flatten() {
                let piece = piece.replace(minus.as_str(), "").replace(plus.as_str(), "");
                let piece = piece.trim_matches(|c: char| c.is_whitespace() || matches!(c, '(' | ')'));
                if !piece.is_empty() && !literals.iter().any(|l| l == piece) {
                    literals.push(piece.to_owned());
                }
            }
        }
        literals.sort_by_key(|l| std::cmp::Reverse(l.chars().count()));
        Symbols {
            decimal,
            group,
            minus,
            plus,
            literals,
        }
    }

    /// `d` written by the format's body.
    fn body(&self, d: &mut Decimal) -> String {
        match &self.body {
            Body::Decimal => self.decimal.format(d).write_to_string().into_owned(),
            Body::Percent { plain, signed } => {
                let f = if d.sign() == Sign::Positive { signed } else { plain };
                f.format(d).write_to_string().into_owned()
            }
            Body::Currency(f) => f.format_fixed_decimal(d).write_to_string().into_owned(),
            Body::Unit(u) => {
                let rules = &u.rules;
                let abs = d.clone().with_sign(Sign::None);
                let number = self.decimal.format(d).write_to_string().into_owned();
                let pattern = u.names.get().get((&abs).into(), rules);
                pattern.interpolate([number]).write_to_string().into_owned()
            }
        }
    }
}

fn decimal_formatter(locale: &Locale, grouping: GroupingStrategy) -> DecimalFormatter {
    let mut options = DecimalFormatterOptions::default();
    options.grouping_strategy = Some(grouping);
    DecimalFormatter::try_new(locale.into(), options)
        .or_else(|_| DecimalFormatter::try_new(Locale::UNKNOWN.into(), options))
        .unwrap_or_else(|_| unreachable_formatter())
}

/// The root locale's formatter is compiled in; it cannot fail to load.
fn unreachable_formatter() -> DecimalFormatter {
    #[expect(clippy::expect_used, reason = "the root locale's decimal data is compiled in")]
    DecimalFormatter::try_new(Default::default(), DecimalFormatterOptions::default())
        .expect("compiled root decimal data")
}

impl NumberFormat {
    /// `{ style: 'percent' }` in en-US: whole percents.
    #[must_use]
    pub fn percent() -> Self {
        Self {
            style: NumberStyle::Percent,
            max_fraction: 0,
            ..Self::default()
        }
    }

    /// `{ style: 'currency', currency: code, currencyDisplay: 'code' }` in
    /// en-US, with the currency's own decimals.
    #[must_use]
    pub fn currency(code: &'static str, accounting: bool) -> Self {
        Self {
            style: NumberStyle::Currency { code, accounting },
            min_fraction: 2,
            max_fraction: 2,
            ..Self::default()
        }
    }

    /// `{ style: 'unit', unit, unitDisplay: width }` in en-US.
    #[must_use]
    pub fn unit(unit: &'static str, width: UnitWidth) -> Self {
        Self {
            style: NumberStyle::Unit { unit, width },
            ..Self::default()
        }
    }

    /// This format in `locale`.
    #[must_use]
    pub fn in_locale(self, locale: Locale) -> Self {
        Self { locale, ..self }
    }

    fn resolved(&self) -> Rc<Resolved> {
        let key = (self.locale.to_string(), self.style);
        RESOLVED.with(|r| {
            Rc::clone(
                r.borrow_mut()
                    .entry(key)
                    .or_insert_with(|| Rc::new(Resolved::new(&self.locale, self.style))),
            )
        })
    }

    /// `v` as a decimal this format writes: scaled, rounded, padded, signed.
    fn decimal(&self, v: f64) -> Option<Decimal> {
        let mut d = Decimal::try_from_f64(v, FloatPrecision::RoundTrip).ok()?;
        if self.style == NumberStyle::Percent {
            d.multiply_pow10(2);
            d.absolute.trim_start();
        }
        let max = i16::try_from(self.max_fraction.max(self.min_fraction)).unwrap_or(i16::MAX);
        let min = i16::try_from(self.min_fraction).unwrap_or(i16::MAX);
        d.round(max.saturating_neg());
        d.trim_end();
        d.pad_end(min.saturating_neg());
        d.apply_sign_display(match self.sign {
            SignDisplay::Auto => fixed_decimal::SignDisplay::Negative,
            SignDisplay::ExceptZero => fixed_decimal::SignDisplay::ExceptZero,
        });
        Some(d)
    }

    /// `v` as `Intl.NumberFormat(locale, options)` writes it; empty for NaN.
    #[must_use]
    pub fn format(&self, v: f64) -> String {
        if !v.is_finite() {
            return String::new();
        }
        let Some(mut d) = self.decimal(v) else {
            return String::new();
        };
        self.resolved().body(&mut d)
    }

    /// The text without what this format adds around the number, its
    /// separators turned plain (no group, `.` for the decimal, `-` for the
    /// minus), its digits kept; and whether accounting parentheses made it
    /// negative.
    fn bare(&self, text: &str) -> (String, bool) {
        let r = self.resolved();
        let s = &r.symbols;
        let mut t: String = text.chars().filter(|c| !is_bidi_mark(*c)).collect();
        for l in &s.literals {
            t = t.replace(l.as_str(), "");
        }
        let negative = t.contains('(') && t.contains(')');
        t.retain(|c| !matches!(c, '(' | ')') && !c.is_whitespace());
        if !s.minus.is_empty() && s.minus != "-" {
            t = t.replace(s.minus.as_str(), "-");
        }
        if !s.plus.is_empty() && s.plus != "+" {
            t = t.replace(s.plus.as_str(), "+");
        }
        let group = s.group.trim();
        let mut out = String::with_capacity(t.len());
        let mut rest = t.as_str();
        while let Some(c) = rest.chars().next() {
            if !s.decimal.is_empty() && rest.starts_with(s.decimal.as_str()) {
                out.push('.');
                rest = rest.get(s.decimal.len()..).unwrap_or_default();
            } else if !group.is_empty() && rest.starts_with(group) {
                rest = rest.get(group.len()..).unwrap_or_default();
            } else {
                out.push(c);
                rest = rest.get(c.len_utf8()..).unwrap_or_default();
            }
        }
        (out, negative)
    }

    /// The number in `text` as this format writes it, or plainly typed;
    /// none for text with no digit or that does not parse.
    #[must_use]
    pub fn parse(&self, text: &str) -> Option<f64> {
        let (bare, negative) = self.bare(text);
        if !bare.chars().any(|c| digit(c).is_some()) {
            return None;
        }
        let mut plain = String::with_capacity(bare.len());
        for c in bare.chars() {
            match digit(c) {
                Some(d) => plain.push(char::from_digit(d, 10)?),
                None if matches!(c, '.' | '-' | '+') => plain.push(c),
                None => return None,
            }
        }
        let v = plain.parse::<f64>().ok().filter(|v| v.is_finite())?;
        let v = if negative { -v.abs() } else { v };
        Some(if self.style == NumberStyle::Percent {
            v / 100.0
        } else {
            v
        })
    }

    /// Whether `text` is a number in progress in this format: what may stand
    /// in the input while typing (reka's `isValidPartialNumber`). A minus
    /// only when `negative` allows one.
    #[must_use]
    pub fn partial(&self, text: &str, negative: bool) -> bool {
        let (bare, parens) = self.bare(text);
        if parens && !negative {
            return false;
        }
        let decimals = match self.style {
            // A currency keeps its own decimals: a point may be typed.
            NumberStyle::Currency { .. } => true,
            _ => self.max_fraction > 0,
        };
        let mut chars = bare.chars().peekable();
        if let Some(sign) = chars.peek().copied()
            && matches!(sign, '-' | '+')
        {
            if sign == '-' && !negative {
                return false;
            }
            chars.next();
        }
        let mut point = false;
        for c in chars {
            match c {
                '.' if !point && decimals => point = true,
                c if digit(c).is_some() => {}
                _ => return false,
            }
        }
        true
    }

    /// `v` as this format holds it: rounded to its decimals (format, then
    /// parse; `NumberFieldRoot.vue:165,231,251`).
    #[must_use]
    pub fn normalise(&self, v: f64) -> f64 {
        self.parse(&self.format(v)).unwrap_or(v)
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberFormat, SignDisplay, UnitWidth, digit};

    fn de() -> icu_locale_core::Locale {
        icu_locale_core::locale!("de-DE")
    }

    #[test]
    fn en_us_writes_as_intl_does() {
        let plain = NumberFormat::default();
        assert_eq!(plain.format(1234.5), "1,234.5");
        assert_eq!(plain.format(-0.0), "0");
        assert_eq!(plain.format(0.12345), "0.123");
        assert_eq!(plain.format(f64::NAN), "");
        assert_eq!(NumberFormat::percent().format(0.05), "5%");
        assert_eq!(NumberFormat::currency("EUR", false).format(7.0), "EUR\u{a0}7.00");
        assert_eq!(NumberFormat::currency("EUR", true).format(-5.0), "(EUR\u{a0}5.00)");
        let signed = NumberFormat {
            sign: SignDisplay::ExceptZero,
            ..NumberFormat::default()
        };
        assert_eq!(signed.format(5.0), "+5");
        assert_eq!(signed.format(0.0), "0");
        assert_eq!(signed.format(-5.0), "-5");
        // The docs' demos, as Chrome writes them.
        assert_eq!(NumberFormat::currency("EUR", true).format(1500.0), "EUR\u{a0}1,500.00");
        let decimal = NumberFormat {
            sign: SignDisplay::ExceptZero,
            min_fraction: 1,
            ..NumberFormat::default()
        };
        assert_eq!(decimal.format(5.0), "+5.0");
        assert_eq!(plain.format(1236.0), "1,236");
        assert_eq!(plain.format(-5.0), "-5");
        assert_eq!(NumberFormat::unit("inch", UnitWidth::Long).format(5.0), "5 inches");
        assert_eq!(NumberFormat::unit("inch", UnitWidth::Long).format(1.0), "1 inch");
        assert_eq!(NumberFormat::unit("minute", UnitWidth::Short).format(5.0), "5 min");
    }

    #[test]
    fn a_locale_writes_and_reads_its_own_separators_and_digits() {
        let plain = NumberFormat::default().in_locale(de());
        assert_eq!(plain.format(1234.5), "1.234,5");
        assert_eq!(plain.parse("1.234"), Some(1234.0), "de-DE: `.` groups");
        assert_eq!(plain.parse("1,5"), Some(1.5));
        assert!(plain.partial("12,", true) && !plain.partial("1,2,", true));
        let eur = NumberFormat::currency("EUR", false).in_locale(de());
        assert_eq!(eur.format(7.0), "7,00\u{a0}EUR");
        assert_eq!(eur.parse("7,00 EUR"), Some(7.0));
        assert_eq!(NumberFormat::percent().in_locale(de()).format(0.05), "5\u{a0}%");
        let ar = NumberFormat::default().in_locale(icu_locale_core::locale!("ar-EG"));
        assert_eq!(ar.format(123.0), "١٢٣");
        assert_eq!(ar.parse("١٢٣٫٥"), Some(123.5));
        let inr = NumberFormat::default().in_locale(icu_locale_core::locale!("en-IN"));
        assert_eq!(inr.format(1_234_567.0), "12,34,567");
        let km = NumberFormat::unit("kilometer", UnitWidth::Short);
        assert_eq!(km.parse("12 km"), Some(12.0), "the unit's name is dropped");
        let long = NumberFormat::unit("inch", UnitWidth::Long);
        assert_eq!(long.parse("2 inches"), Some(2.0));
        assert_eq!(long.parse("1 inch"), Some(1.0));
    }

    #[test]
    fn digits_of_every_script_and_the_han_numerals() {
        assert_eq!(digit('٣'), Some(3));
        assert_eq!(digit('五'), Some(5));
        assert_eq!(digit('７'), Some(7));
        assert_eq!(digit('a'), None);
        assert_eq!(NumberFormat::default().parse("١٢٣"), Some(123.0));
        assert_eq!(NumberFormat::default().parse("一二三"), Some(123.0));
        assert_eq!(NumberFormat::percent().parse("٢٥"), Some(0.25));
        assert_eq!(NumberFormat::default().parse("1,234.5"), Some(1234.5));
        assert_eq!(NumberFormat::currency("EUR", true).parse("(EUR 5.00)"), Some(-5.0));
        let f = NumberFormat::default();
        assert!(f.partial("-", true) && !f.partial("-", false), "a minus only below 0");
        assert!(f.partial("12.", true) && !f.partial("1.2.", true) && !f.partial("1a", true));
    }
}
