//! What a calendar reads out in a locale (reka-ui's `locale`, formatted by
//! `Intl`): month and weekday names, digits and the first day of the week
//! (the direction is the calendar's `dir`, not the locale's). Holds the locales a demo shows; the strings
//! are Chrome's `Intl` (ICU) output for them.

use crate::calendar::CalendarDate;
use crate::calendar_system::{CalendarSystem, Era};

/// A locale a calendar can read out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum CalendarLocale {
    /// `en-US`: Sunday first, every system's months by their English names.
    #[default]
    EnUs,
    /// `fa-IR`: Saturday first, Persian digits; Persian,
    /// Gregorian and Islamic months by their Persian names.
    FaIr,
}

/// reka-ui's `weekdayFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WeekdayFormat {
    /// "S" (reka-ui's default).
    #[default]
    Narrow,
    /// "Sun".
    Short,
    /// "Sunday".
    Long,
}

/// `Intl`'s `month: 'long'` or `'short'`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonthWidth {
    Long,
    Short,
}

const EN_GREGORIAN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const EN_PERSIAN: [&str; 12] = [
    "Farvardin",
    "Ordibehesht",
    "Khordad",
    "Tir",
    "Mordad",
    "Shahrivar",
    "Mehr",
    "Aban",
    "Azar",
    "Dey",
    "Bahman",
    "Esfand",
];
const EN_INDIAN: [&str; 12] = [
    "Chaitra",
    "Vaisakha",
    "Jyaistha",
    "Asadha",
    "Sravana",
    "Bhadra",
    "Asvina",
    "Kartika",
    "Agrahayana",
    "Pausa",
    "Magha",
    "Phalguna",
];
const EN_ISLAMIC: [&str; 12] = [
    "Muharram",
    "Safar",
    "Rabiʻ I",
    "Rabiʻ II",
    "Jumada I",
    "Jumada II",
    "Rajab",
    "Shaʻban",
    "Ramadan",
    "Shawwal",
    "Dhuʻl-Qiʻdah",
    "Dhuʻl-Hijjah",
];
const EN_ISLAMIC_SHORT: [&str; 12] = [
    "Muh.",
    "Saf.",
    "Rab. I",
    "Rab. II",
    "Jum. I",
    "Jum. II",
    "Raj.",
    "Sha.",
    "Ram.",
    "Shaw.",
    "Dhuʻl-Q.",
    "Dhuʻl-H.",
];
/// A leap year's thirteen; a common year has no Adar I and calls Adar II Adar.
const EN_HEBREW: [&str; 13] = [
    "Tishri", "Heshvan", "Kislev", "Tevet", "Shevat", "Adar I", "Adar II", "Nisan", "Iyar", "Sivan", "Tamuz", "Av",
    "Elul",
];
const EN_COPTIC: [&str; 13] = [
    "Tout",
    "Baba",
    "Hator",
    "Kiahk",
    "Toba",
    "Amshir",
    "Baramhat",
    "Baramouda",
    "Bashans",
    "Paona",
    "Epep",
    "Mesra",
    "Nasie",
];
const EN_ETHIOPIC: [&str; 13] = [
    "Meskerem", "Tekemt", "Hedar", "Tahsas", "Ter", "Yekatit", "Megabit", "Miazia", "Genbot", "Sene", "Hamle",
    "Nehasse", "Pagumen",
];
const FA_PERSIAN: [&str; 12] = [
    "فروردین",
    "اردیبهشت",
    "خرداد",
    "تیر",
    "مرداد",
    "شهریور",
    "مهر",
    "آبان",
    "آذر",
    "دی",
    "بهمن",
    "اسفند",
];
const FA_GREGORIAN: [&str; 12] = [
    "ژانویه",
    "فوریه",
    "مارس",
    "آوریل",
    "مه",
    "ژوئن",
    "ژوئیه",
    "اوت",
    "سپتامبر",
    "اکتبر",
    "نوامبر",
    "دسامبر",
];
const FA_ISLAMIC: [&str; 12] = [
    "محرم",
    "صفر",
    "ربیع‌الاول",
    "ربیع‌الثانی",
    "جمادی‌الاول",
    "جمادی‌الثانی",
    "رجب",
    "شعبان",
    "رمضان",
    "شوال",
    "ذیقعده",
    "ذیحجه",
];

/// Sunday first, whatever day a locale's week starts on.
const EN_WEEKDAYS_NARROW: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];
const EN_WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const FA_WEEKDAYS_NARROW: [&str; 7] = ["ی", "د", "س", "چ", "پ", "ج", "ش"];
const FA_WEEKDAYS: [&str; 7] = ["یکشنبه", "دوشنبه", "سه‌شنبه", "چهارشنبه", "پنجشنبه", "جمعه", "شنبه"];

impl CalendarLocale {
    /// The locale a BCP 47 tag names (`"en-US"`, `"fa-IR"`), if it is one of these.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.to_ascii_lowercase().as_str() {
            "en" | "en-us" => Some(Self::EnUs),
            "fa" | "fa-ir" => Some(Self::FaIr),
            _ => None,
        }
    }

    /// The weekday a week starts on, 0 for Sunday (reka-ui's `getWeekStartsOn`).
    #[must_use]
    pub fn first_weekday(self) -> u8 {
        match self {
            Self::EnUs => 0,
            Self::FaIr => 6,
        }
    }

    /// `n` in the locale's digits, without grouping (`toLocaleString` of a day or year).
    #[must_use]
    pub fn number(self, n: i64) -> String {
        let latin = n.to_string();
        match self {
            Self::EnUs => latin,
            Self::FaIr => latin
                .chars()
                .map(|c| {
                    c.to_digit(10)
                        .and_then(|d| char::from_u32(0x06F0_u32.saturating_add(d)))
                        .unwrap_or(c)
                })
                .collect(),
        }
    }

    /// The name of `weekday` (0 for Sunday).
    #[must_use]
    pub fn weekday(self, weekday: u8, format: WeekdayFormat) -> &'static str {
        let names = match (self, format) {
            (Self::EnUs, WeekdayFormat::Narrow) => &EN_WEEKDAYS_NARROW,
            (Self::EnUs, WeekdayFormat::Short) => &EN_WEEKDAYS_SHORT,
            (Self::EnUs, WeekdayFormat::Long) => &EN_WEEKDAYS_LONG,
            (Self::FaIr, WeekdayFormat::Narrow) => &FA_WEEKDAYS_NARROW,
            (Self::FaIr, WeekdayFormat::Short | WeekdayFormat::Long) => &FA_WEEKDAYS,
        };
        names.get(usize::from(weekday)).copied().unwrap_or_default()
    }

    /// The name of `date`'s month in `system`.
    #[must_use]
    pub fn month(self, system: CalendarSystem, date: CalendarDate, width: MonthWidth) -> &'static str {
        let f = system.fields(date);
        let index = usize::from(f.month.saturating_sub(1));
        let from = |names: &[&'static str]| names.get(index).copied().unwrap_or_default();
        match (self, system) {
            (Self::FaIr, CalendarSystem::Persian) => from(&FA_PERSIAN),
            (Self::FaIr, CalendarSystem::Gregorian) => from(&FA_GREGORIAN),
            (
                Self::FaIr,
                CalendarSystem::IslamicUmalqura | CalendarSystem::IslamicCivil | CalendarSystem::IslamicTabular,
            ) => from(&FA_ISLAMIC),
            (
                _,
                CalendarSystem::Gregorian
                | CalendarSystem::Japanese
                | CalendarSystem::Buddhist
                | CalendarSystem::Taiwan,
            ) => {
                let name = from(&EN_GREGORIAN);
                match width {
                    MonthWidth::Long => name,
                    MonthWidth::Short => name.get(..3).unwrap_or(name),
                }
            }
            (_, CalendarSystem::Persian) => from(&EN_PERSIAN),
            (_, CalendarSystem::Indian) => from(&EN_INDIAN),
            (_, CalendarSystem::IslamicUmalqura | CalendarSystem::IslamicCivil | CalendarSystem::IslamicTabular) => {
                match width {
                    MonthWidth::Long => from(&EN_ISLAMIC),
                    MonthWidth::Short => from(&EN_ISLAMIC_SHORT),
                }
            }
            (_, CalendarSystem::Hebrew) if system.months_in_year(f.year) == 13 => from(&EN_HEBREW),
            (_, CalendarSystem::Hebrew) => match f.month {
                6 => "Adar",
                m if m > 6 => EN_HEBREW.get(usize::from(m)).copied().unwrap_or_default(),
                _ => from(&EN_HEBREW),
            },
            (_, CalendarSystem::Coptic) => from(&EN_COPTIC),
            (_, CalendarSystem::Ethiopic | CalendarSystem::EthiopicAmeteAlem) => from(&EN_ETHIOPIC),
        }
    }

    /// `date`'s year as `Intl`'s `year: 'numeric'` writes it: "2026",
    /// "8 Reiwa", "۱۴۰۵".
    #[must_use]
    pub fn year(self, system: CalendarSystem, date: CalendarDate) -> String {
        let (year, era) = system.era(date);
        let year = self.number(i64::from(year));
        match (self, era) {
            (Self::FaIr, Era::Ah) => format!("{year} ه\u{200d}.ق."),
            (Self::FaIr, _) | (Self::EnUs, Era::Unnamed) => year,
            (Self::EnUs, era) => format!("{year} {}", en_era(era)),
        }
    }

    /// The heading: `Intl`'s `month: 'long', year: 'numeric'` ("November 2026",
    /// "Aban 1405 AP", "۱۴۰۵ آبان").
    #[must_use]
    pub fn month_and_year(self, system: CalendarSystem, date: CalendarDate) -> String {
        let month = self.month(system, date, MonthWidth::Long);
        match (self, system) {
            (Self::FaIr, CalendarSystem::Persian) => format!("{} {month}", self.year(system, date)),
            (Self::FaIr, _) => format!("{month} {}", self.number(i64::from(system.era(date).0))),
            (Self::EnUs, _) => format!("{month} {}", self.year(system, date)),
        }
    }
}

fn en_era(era: Era) -> &'static str {
    match era {
        Era::Unnamed => "",
        Era::Meiji => "Meiji",
        Era::Taisho => "Taishō",
        Era::Showa => "Shōwa",
        Era::Heisei => "Heisei",
        Era::Reiwa => "Reiwa",
        Era::Be => "BE",
        Era::Minguo => "Minguo",
        Era::BeforeMinguo => "B.R.O.C.",
        Era::Ap => "AP",
        Era::Saka => "Saka",
        Era::Ah => "AH",
        Era::Era0 => "ERA0",
        Era::Era1 => "ERA1",
    }
}
