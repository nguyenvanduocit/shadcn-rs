//! What a date reads out in a locale (reka-ui's `locale`, formatted by
//! `Intl`): month and weekday names, digits, the first day of the week, a
//! day's full name for assistive technology, and the order and placeholders
//! of a date field's segments (the direction is the caller's `dir`, not the
//! locale's). Holds the locales the demos show and Vietnamese; the strings
//! are Chrome's `Intl` (ICU) output for them.

use super::CalendarDate;
use super::system::{CalendarSystem, Era};

/// A locale a calendar can read out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum CalendarLocale {
    /// `en-US`: Sunday first, every system's months by their English names.
    #[default]
    EnUs,
    /// `fa-IR`: Saturday first, Persian digits; Persian,
    /// Gregorian and Islamic months by their Persian names.
    FaIr,
    /// `vi-VN`: Monday first, Latin digits, Gregorian months as
    /// "tháng 1"; other systems' months by their English names.
    Vi,
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
const VI_WEEKDAYS_NARROW: [&str; 7] = ["CN", "T2", "T3", "T4", "T5", "T6", "T7"];
const VI_WEEKDAYS_SHORT: [&str; 7] = ["CN", "Th 2", "Th 3", "Th 4", "Th 5", "Th 6", "Th 7"];
const VI_WEEKDAYS_LONG: [&str; 7] = [
    "Chủ Nhật",
    "Thứ Hai",
    "Thứ Ba",
    "Thứ Tư",
    "Thứ Năm",
    "Thứ Sáu",
    "Thứ Bảy",
];
const VI_MONTHS: [&str; 12] = [
    "tháng 1",
    "tháng 2",
    "tháng 3",
    "tháng 4",
    "tháng 5",
    "tháng 6",
    "tháng 7",
    "tháng 8",
    "tháng 9",
    "tháng 10",
    "tháng 11",
    "tháng 12",
];
const VI_MONTHS_SHORT: [&str; 12] = [
    "thg 1", "thg 2", "thg 3", "thg 4", "thg 5", "thg 6", "thg 7", "thg 8", "thg 9", "thg 10", "thg 11", "thg 12",
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
            "vi" | "vi-vn" => Some(Self::Vi),
            _ => None,
        }
    }

    /// The weekday a week starts on, 0 for Sunday (reka-ui's `getWeekStartsOn`).
    #[must_use]
    pub fn first_weekday(self) -> u8 {
        match self {
            Self::EnUs => 0,
            Self::FaIr => 6,
            Self::Vi => 1,
        }
    }

    /// `n` in the locale's digits, without grouping (`toLocaleString` of a day or year).
    #[must_use]
    pub fn number(self, n: i64) -> String {
        let latin = n.to_string();
        match self {
            Self::EnUs | Self::Vi => latin,
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
            (Self::Vi, WeekdayFormat::Narrow) => &VI_WEEKDAYS_NARROW,
            (Self::Vi, WeekdayFormat::Short) => &VI_WEEKDAYS_SHORT,
            (Self::Vi, WeekdayFormat::Long) => &VI_WEEKDAYS_LONG,
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
                Self::Vi,
                CalendarSystem::Gregorian
                | CalendarSystem::Japanese
                | CalendarSystem::Buddhist
                | CalendarSystem::Taiwan,
            ) => match width {
                MonthWidth::Long => from(&VI_MONTHS),
                MonthWidth::Short => from(&VI_MONTHS_SHORT),
            },
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
            (Self::FaIr, _) | (Self::EnUs | Self::Vi, Era::Unnamed) => year,
            (Self::EnUs | Self::Vi, era) => format!("{year} {}", en_era(era)),
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
            (Self::Vi, _) => format!("{month} năm {}", self.year(system, date)),
            (Self::EnUs, _) => format!("{month} {}", self.year(system, date)),
        }
    }

    /// `date` named in full for assistive technology, as reka's day trigger
    /// labels itself (`Intl`'s `weekday: 'long', month: 'long', day:
    /// 'numeric', year: 'numeric'`): "Sunday, January 20, 1980",
    /// "Chủ Nhật, 20 tháng 1, 1980", "یکشنبه ۲۰ ژانویه ۱۹۸۰".
    #[must_use]
    pub fn day_label(self, system: CalendarSystem, date: CalendarDate) -> String {
        let weekday = self.weekday(date.weekday(), WeekdayFormat::Long);
        let month = self.month(system, date, MonthWidth::Long);
        let day = self.number(i64::from(system.fields(date).day));
        let year = self.year(system, date);
        match self {
            Self::EnUs => format!("{weekday}, {month} {day}, {year}"),
            Self::Vi => format!("{weekday}, {day} {month}, {year}"),
            Self::FaIr => format!("{weekday} {day} {month} {year}"),
        }
    }

    /// The order of a date's segments as the locale writes a short date
    /// (`formatToParts`): month, day, year in `en-US`; day, month, year in
    /// `vi-VN`; year, month, day in `fa-IR`.
    #[must_use]
    pub fn date_order(self) -> [DatePart; 3] {
        match self {
            Self::EnUs => [DatePart::Month, DatePart::Day, DatePart::Year],
            Self::Vi => [DatePart::Day, DatePart::Month, DatePart::Year],
            Self::FaIr => [DatePart::Year, DatePart::Month, DatePart::Day],
        }
    }

    /// What goes between a date's segments ("/" in each locale here).
    #[must_use]
    pub fn date_separator(self) -> &'static str {
        "/"
    }

    /// Whether the locale tells the time on a 12-hour clock with AM and PM
    /// (`hourCycle` `h12`): `en-US` does, `vi-VN` and `fa-IR` do not.
    #[must_use]
    pub fn twelve_hour(self) -> bool {
        matches!(self, Self::EnUs)
    }

    /// Whether the day period comes before the hour on a 12-hour clock
    /// (`formatToParts`: `ko-KR` writes "오후 2:30"); after it in every
    /// locale here ("2:30 PM").
    #[must_use]
    pub fn day_period_first(self) -> bool {
        match self {
            Self::EnUs | Self::Vi | Self::FaIr => false,
        }
    }

    /// What an empty date segment shows: Chrome's `<input type="date">`
    /// placeholders, `mm`, `dd`, `yyyy` in English. reka has none for
    /// Vietnamese (its table has no `vi`, so it shows English's);
    /// Chrome's `vi` input shows `dd`, `mm`, `yyyy`, which is what is
    /// kept, so a Vietnamese reader sees the letters of the format it
    /// reads ("dd/mm/yyyy"), as on the web (ADR-0025, group 7).
    #[must_use]
    pub fn placeholder(self, part: DatePart) -> &'static str {
        match (self, part) {
            (Self::EnUs | Self::Vi, DatePart::Year) => "yyyy",
            (Self::EnUs | Self::Vi, DatePart::Month) => "mm",
            (Self::EnUs | Self::Vi, DatePart::Day) => "dd",
            (Self::FaIr, DatePart::Year) => "سال",
            (Self::FaIr, DatePart::Month) => "ماه",
            (Self::FaIr, DatePart::Day) => "روز",
        }
    }

    /// The ISO-style week rule the locale numbers weeks by: a week starting
    /// on Monday whose first week holds January 4 (`en-GB`, `vi`, `fa`'s
    /// Gregorian numbering aside), else weeks from Sunday whose first holds
    /// January 1 (`en-US`) (`date/calendar.ts`'s `getWeekNumber`).
    #[must_use]
    pub fn iso_weeks(self) -> bool {
        matches!(self, Self::Vi)
    }
}

/// One of a date's three parts, as [`CalendarLocale::date_order`] orders them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DatePart {
    Year,
    Month,
    Day,
}

/// What assistive technology is told of a calendar and of a date or time
/// field, in one language (research §2.2 item 15, §11 "Chuỗi AT"): reka
/// writes these in English only ("Event Date", "Previous page", "day,",
/// "Empty"…). The field names are Chrome's (`IDS_AX_*_FIELD_TEXT`), since
/// Chrome's date and time inputs are what the verdict compares against; the
/// calendar's are reka's. An app gives its own for a language not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateStrings {
    /// The calendar's own name, before its heading (reka's `calendarLabel`).
    pub calendar: String,
    pub previous_page: String,
    pub next_page: String,
    pub year: String,
    pub month: String,
    pub day: String,
    pub hour: String,
    pub minute: String,
    pub second: String,
    pub day_period: String,
    /// An empty segment's value.
    pub blank: String,
    /// A picker's trigger, as Chrome names its date input's button (reka's
    /// trigger has no name).
    pub show_picker: String,
}

impl CalendarLocale {
    /// The locale's [`DateStrings`].
    #[must_use]
    pub fn strings(self) -> DateStrings {
        let s = |v: [&str; 12]| {
            let [
                calendar,
                previous_page,
                next_page,
                year,
                month,
                day,
                hour,
                minute,
                second,
                day_period,
                blank,
                show_picker,
            ] = v.map(str::to_owned);
            DateStrings {
                calendar,
                previous_page,
                next_page,
                year,
                month,
                day,
                hour,
                minute,
                second,
                day_period,
                blank,
                show_picker,
            }
        };
        match self {
            Self::EnUs => s([
                "Event Date",
                "Previous page",
                "Next page",
                "Year",
                "Month",
                "Day",
                "Hours",
                "Minutes",
                "Seconds",
                "AM/PM",
                "Blank",
                "Show date picker",
            ]),
            Self::Vi => s([
                "Ngày sự kiện",
                "Trang trước",
                "Trang sau",
                "Năm",
                "Tháng",
                "Ngày",
                "Giờ",
                "Phút",
                "Giây",
                "SA/CH",
                "Trống",
                "Hiển thị bộ chọn ngày",
            ]),
            Self::FaIr => s([
                "تاریخ رویداد",
                "صفحهٔ قبل",
                "صفحهٔ بعد",
                "سال",
                "ماه",
                "روز",
                "ساعت",
                "دقیقه",
                "ثانیه",
                "ق.ظ./ب.ظ.",
                "خالی",
                "نمایش انتخابگر تاریخ",
            ]),
        }
    }

    /// The two halves of a 12-hour day as the locale writes them (`AM`,
    /// `PM`; `SA`, `CH`): translated as Chrome's time input shows them,
    /// where reka always writes "AM" and "PM" (ADR-0025, group 7).
    #[must_use]
    pub fn day_periods(self) -> [&'static str; 2] {
        match self {
            Self::EnUs => ["AM", "PM"],
            Self::Vi => ["SA", "CH"],
            Self::FaIr => ["ق.ظ.", "ب.ظ."],
        }
    }
}

/// The value of a typed digit: ASCII `0`–`9`, and the decimal digits of the
/// scripts a locale here writes numbers in (Arabic-Indic, Persian, fullwidth,
/// Devanagari), so a Persian keyboard types into a field as a Latin one does.
/// reka takes ASCII only, while it shows the locale's digits (ADR-0025,
/// group 7).
#[must_use]
pub fn digit_value(c: char) -> Option<u8> {
    let zero = match c {
        '0'..='9' => '0',
        '\u{0660}'..='\u{0669}' => '\u{0660}',
        '\u{06F0}'..='\u{06F9}' => '\u{06F0}',
        '\u{0966}'..='\u{096F}' => '\u{0966}',
        '\u{FF10}'..='\u{FF19}' => '\u{FF10}',
        _ => return None,
    };
    u8::try_from(u32::from(c).saturating_sub(u32::from(zero))).ok()
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
