//! Calendar systems (`@internationalized/date`'s calendars): how a day is
//! counted into years, months and days. A [`CalendarDate`] is one day
//! whatever the system; a system reads its fields and finds the day that
//! has them. The arithmetic is ICU's, by way of `@internationalized/date`,
//! on Julian day numbers.

use std::num::Saturating;

use super::{CalendarDate, quot, rem, s};

/// The systems `createCalendar` knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum CalendarSystem {
    #[default]
    Gregorian,
    /// Gregorian months, years counted in the emperor's era.
    Japanese,
    /// Gregorian months, years from the Buddha's death (543 BC).
    Buddhist,
    /// Gregorian months, years from the Republic of China (1912).
    Taiwan,
    /// The solar Hijri calendar of Iran.
    Persian,
    /// The Indian national (Saka) calendar.
    Indian,
    /// Saudi Arabia's Islamic calendar, by table from 1300 to 1600 AH.
    IslamicUmalqura,
    /// The arithmetic Islamic calendar from the civil epoch (16 July 622).
    IslamicCivil,
    /// The arithmetic Islamic calendar from the astronomical epoch (15 July 622).
    IslamicTabular,
    /// The Hebrew calendar: 12 months, 13 in a leap year.
    Hebrew,
    /// 12 months of 30 days and a 13th of 5 or 6.
    Coptic,
    /// As Coptic, from the incarnation era (Amete Mihret).
    Ethiopic,
    /// As Ethiopic, years from creation (Amete Alem).
    EthiopicAmeteAlem,
}

/// A day as a system counts it. `year` is the year the arithmetic runs on:
/// the Gregorian one for the systems that only rename years, and one that
/// may be zero or negative before a system's epoch; see [`CalendarSystem::era`]
/// for the year a reader is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemDate {
    pub year: i32,
    /// From 1.
    pub month: u8,
    pub day: u8,
}

/// The era a year is shown in, as `Intl` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    /// Not shown (Gregorian AD, Hebrew AM).
    Unnamed,
    Meiji,
    Taisho,
    Showa,
    Heisei,
    Reiwa,
    /// Buddhist era.
    Be,
    /// Republic of China.
    Minguo,
    BeforeMinguo,
    /// Anno Persico.
    Ap,
    Saka,
    /// Anno Hegirae.
    Ah,
    /// ICU's unnamed eras: Coptic before and after Diocletian, Ethiopic
    /// Amete Alem and Amete Mihret.
    Era0,
    Era1,
}

type N = Saturating<i64>;

/// Julian day of 1970-01-01, the day [`CalendarDate`] counts from.
const UNIX_JD: i64 = 2_440_588;
const GREGORIAN_EPOCH: i64 = 1_721_426;
const PERSIAN_EPOCH: i64 = 1_948_320;
/// Days from the start of a Persian year to the start of each month.
const PERSIAN_MONTH_START: [i64; 12] = [0, 31, 62, 93, 124, 155, 186, 216, 246, 276, 306, 336];
const INDIAN_ERA_START: i64 = 78;
const INDIAN_YEAR_START: i64 = 80;
const ISLAMIC_CIVIL_EPOCH: i64 = 1_948_440;
const ISLAMIC_ASTRONOMICAL_EPOCH: i64 = 1_948_439;
const UMALQURA_FIRST_YEAR: i64 = 1300;
const UMALQURA_LAST_YEAR: i64 = 1600;
const UMALQURA_START_DAYS: i64 = 460_322;
/// Per year from 1300 AH, a bit per month (January's highest of twelve):
/// set for 30 days, clear for 29 (`@internationalized/date`'s table).
const UMALQURA_MONTHS: [u16; 301] = [
    0x0aaa, 0x0d54, 0x0ec9, 0x06d4, 0x06ea, 0x036c, 0x0aad, 0x0555, 0x06a9, 0x0792, 0x0ba9, 0x05d4, 0x0ada, 0x055c,
    0x0d2d, 0x0695, 0x074a, 0x0b54, 0x0b6a, 0x05ad, 0x04ae, 0x0a4f, 0x0517, 0x068b, 0x06a5, 0x0ad5, 0x02d6, 0x095b,
    0x049d, 0x0a4d, 0x0d26, 0x0d95, 0x05ac, 0x09b6, 0x02ba, 0x0a5b, 0x052b, 0x0a95, 0x06ca, 0x0ae9, 0x02f4, 0x0976,
    0x02b6, 0x0956, 0x0aca, 0x0ba4, 0x0bd2, 0x05d9, 0x02dc, 0x096d, 0x054d, 0x0aa5, 0x0b52, 0x0ba5, 0x05b4, 0x09b6,
    0x0557, 0x0297, 0x054b, 0x06a3, 0x0752, 0x0b65, 0x056a, 0x0aab, 0x052b, 0x0c95, 0x0d4a, 0x0da5, 0x05ca, 0x0ad6,
    0x0957, 0x04ab, 0x094b, 0x0aa5, 0x0b52, 0x0b6a, 0x0575, 0x0276, 0x08b7, 0x045b, 0x0555, 0x05a9, 0x05b4, 0x09da,
    0x04dd, 0x026e, 0x0936, 0x0aaa, 0x0d54, 0x0db2, 0x05d5, 0x02da, 0x095b, 0x04ab, 0x0a55, 0x0b49, 0x0b64, 0x0b71,
    0x05b4, 0x0ab5, 0x0a55, 0x0d25, 0x0e92, 0x0ec9, 0x06d4, 0x0ae9, 0x096b, 0x04ab, 0x0a93, 0x0d49, 0x0da4, 0x0db2,
    0x0ab9, 0x04ba, 0x0a5b, 0x052b, 0x0a95, 0x0b2a, 0x0b55, 0x055c, 0x04bd, 0x023d, 0x091d, 0x0a95, 0x0b4a, 0x0b5a,
    0x056d, 0x02b6, 0x093b, 0x049b, 0x0655, 0x06a9, 0x0754, 0x0b6a, 0x056c, 0x0aad, 0x0555, 0x0b29, 0x0b92, 0x0ba9,
    0x05d4, 0x0ada, 0x055a, 0x0aab, 0x0595, 0x0749, 0x0764, 0x0baa, 0x05b5, 0x02b6, 0x0a56, 0x0e4d, 0x0b25, 0x0b52,
    0x0b6a, 0x05ad, 0x02ae, 0x092f, 0x0497, 0x064b, 0x06a5, 0x06ac, 0x0ad6, 0x055d, 0x049d, 0x0a4d, 0x0d16, 0x0d95,
    0x05aa, 0x05b5, 0x02da, 0x095b, 0x04ad, 0x0595, 0x06ca, 0x06e4, 0x0aea, 0x04f5, 0x02b6, 0x0956, 0x0aaa, 0x0b54,
    0x0bd2, 0x05d9, 0x02ea, 0x096d, 0x04ad, 0x0a95, 0x0b4a, 0x0ba5, 0x05b2, 0x09b5, 0x04d6, 0x0a97, 0x0547, 0x0693,
    0x0749, 0x0b55, 0x056a, 0x0a6b, 0x052b, 0x0a8b, 0x0d46, 0x0da3, 0x05ca, 0x0ad6, 0x04db, 0x026b, 0x094b, 0x0aa5,
    0x0b52, 0x0b69, 0x0575, 0x0176, 0x08b7, 0x025b, 0x052b, 0x0565, 0x05b4, 0x09da, 0x04ed, 0x016d, 0x08b6, 0x0aa6,
    0x0d52, 0x0da9, 0x05d4, 0x0ada, 0x095b, 0x04ab, 0x0653, 0x0729, 0x0762, 0x0ba9, 0x05b2, 0x0ab5, 0x0555, 0x0b25,
    0x0d92, 0x0ec9, 0x06d2, 0x0ae9, 0x056b, 0x04ab, 0x0a55, 0x0d29, 0x0d54, 0x0daa, 0x09b5, 0x04ba, 0x0a3b, 0x049b,
    0x0a4d, 0x0aaa, 0x0ad5, 0x02da, 0x095d, 0x045e, 0x0a2e, 0x0c9a, 0x0d55, 0x06b2, 0x06b9, 0x04ba, 0x0a5d, 0x052d,
    0x0a95, 0x0b52, 0x0ba8, 0x0bb4, 0x05b9, 0x02da, 0x095a, 0x0b4a, 0x0da4, 0x0ed1, 0x06e8, 0x0b6a, 0x056d, 0x0535,
    0x0695, 0x0d4a, 0x0da8, 0x0dd4, 0x06da, 0x055b, 0x029d, 0x062b, 0x0b15, 0x0b4a, 0x0b95, 0x05aa, 0x0aae, 0x092e,
    0x0c8f, 0x0527, 0x0695, 0x06aa, 0x0ad6, 0x055d, 0x029d,
];
const HEBREW_EPOCH: i64 = 347_997;
const HEBREW_DAY_PARTS: i64 = 24 * 1080;
const HEBREW_MONTH_PARTS: i64 = 29 * HEBREW_DAY_PARTS + 12 * 1080 + 793;
const ETHIOPIC_EPOCH: i64 = 1_723_856;
const COPTIC_EPOCH: i64 = 1_824_665;
const AMETE_MIHRET_DELTA: i64 = 5500;
/// The Japanese eras: the Gregorian day each starts on and the year before its first.
const JAPANESE_ERAS: [((i32, u8, u8), i32, Era); 5] = [
    ((1868, 9, 8), 1867, Era::Meiji),
    ((1912, 7, 30), 1911, Era::Taisho),
    ((1926, 12, 25), 1925, Era::Showa),
    ((1989, 1, 8), 1988, Era::Heisei),
    ((2019, 5, 1), 2018, Era::Reiwa),
];

fn n(v: impl Into<i64>) -> N {
    s(v.into())
}

fn i32_of(v: N) -> i32 {
    i32::try_from(v.0).unwrap_or(if v.0 < 0 { i32::MIN } else { i32::MAX })
}

fn u8_of(v: N) -> u8 {
    u8::try_from(v.0.clamp(0, 255)).unwrap_or(1)
}

impl CalendarSystem {
    /// In the order `CalendarSystems.vue` lists them.
    pub const ALL: [Self; 13] = [
        Self::Gregorian,
        Self::Japanese,
        Self::Buddhist,
        Self::Taiwan,
        Self::Persian,
        Self::Indian,
        Self::IslamicUmalqura,
        Self::IslamicCivil,
        Self::IslamicTabular,
        Self::Hebrew,
        Self::Coptic,
        Self::Ethiopic,
        Self::EthiopicAmeteAlem,
    ];

    /// `createCalendar`'s identifier: `"gregory"`, `"persian"`, …
    #[must_use]
    pub fn identifier(self) -> &'static str {
        match self {
            Self::Gregorian => "gregory",
            Self::Japanese => "japanese",
            Self::Buddhist => "buddhist",
            Self::Taiwan => "roc",
            Self::Persian => "persian",
            Self::Indian => "indian",
            Self::IslamicUmalqura => "islamic-umalqura",
            Self::IslamicCivil => "islamic-civil",
            Self::IslamicTabular => "islamic-tbla",
            Self::Hebrew => "hebrew",
            Self::Coptic => "coptic",
            Self::Ethiopic => "ethiopic",
            Self::EthiopicAmeteAlem => "ethioaa",
        }
    }

    /// The system an identifier names.
    #[must_use]
    pub fn from_identifier(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.identifier() == id)
    }

    /// `date`'s year, month and day in this system.
    #[must_use]
    pub fn fields(self, date: CalendarDate) -> SystemDate {
        let jd = n(date.days()) + n(UNIX_JD);
        let (y, m, d) = match self {
            Self::Gregorian | Self::Japanese | Self::Buddhist | Self::Taiwan => {
                return SystemDate {
                    year: date.year(),
                    month: date.month(),
                    day: date.day(),
                };
            }
            Self::Persian => persian_from_jd(jd),
            Self::Indian => indian_from_jd(jd),
            Self::IslamicUmalqura => umalqura_from_jd(jd),
            Self::IslamicCivil => islamic_from_jd(ISLAMIC_CIVIL_EPOCH, jd),
            Self::IslamicTabular => islamic_from_jd(ISLAMIC_ASTRONOMICAL_EPOCH, jd),
            Self::Hebrew => hebrew_from_jd(jd),
            Self::Coptic => ce_from_jd(COPTIC_EPOCH, jd),
            Self::Ethiopic => ce_from_jd(ETHIOPIC_EPOCH, jd),
            Self::EthiopicAmeteAlem => {
                let (y, m, d) = ce_from_jd(ETHIOPIC_EPOCH, jd);
                (y + n(AMETE_MIHRET_DELTA), m, d)
            }
        };
        SystemDate {
            year: i32_of(y),
            month: u8_of(m),
            day: u8_of(d),
        }
    }

    /// The day with these fields; the month is held to the year's and the
    /// day to the month's, and the day to [`CalendarDate`]'s years 1 to 9999.
    #[must_use]
    pub fn date(self, year: i32, month: u8, day: u8) -> CalendarDate {
        let month = month.max(1).min(self.months_in_year(year));
        let day = day.max(1).min(self.days_in_month(year, month));
        let (y, m, d) = (n(year), n(month), n(day));
        let jd = match self {
            Self::Gregorian | Self::Japanese | Self::Buddhist | Self::Taiwan => gregorian_to_jd(y, m, d),
            Self::Persian => persian_to_jd(y, m, d),
            Self::Indian => indian_to_jd(y, m, d),
            Self::IslamicUmalqura => umalqura_to_jd(y, m, d),
            Self::IslamicCivil => islamic_to_jd(ISLAMIC_CIVIL_EPOCH, y, m, d),
            Self::IslamicTabular => islamic_to_jd(ISLAMIC_ASTRONOMICAL_EPOCH, y, m, d),
            Self::Hebrew => hebrew_to_jd(y, m, d),
            Self::Coptic => ce_to_jd(COPTIC_EPOCH, y, m, d),
            Self::Ethiopic => ce_to_jd(ETHIOPIC_EPOCH, y, m, d),
            Self::EthiopicAmeteAlem => ce_to_jd(ETHIOPIC_EPOCH, y - n(AMETE_MIHRET_DELTA), m, d),
        };
        CalendarDate::from_days((jd - n(UNIX_JD)).0)
    }

    #[must_use]
    pub fn months_in_year(self, year: i32) -> u8 {
        match self {
            Self::Hebrew if hebrew_leap(n(year)) => 13,
            Self::Coptic | Self::Ethiopic | Self::EthiopicAmeteAlem => 13,
            _ => 12,
        }
    }

    #[must_use]
    pub fn days_in_month(self, year: i32, month: u8) -> u8 {
        let (y, m) = (n(year), n(month));
        u8_of(match self {
            Self::Gregorian | Self::Japanese | Self::Buddhist | Self::Taiwan => gregorian_days_in_month(y, m),
            Self::Persian => persian_days_in_month(y, m),
            Self::Indian => indian_days_in_month(y, m),
            Self::IslamicUmalqura if umalqura_covers(y) => umalqura_month_length(y, m),
            Self::IslamicUmalqura | Self::IslamicCivil | Self::IslamicTabular => islamic_days_in_month(y, m),
            Self::Hebrew => hebrew_days_in_month(y, m),
            Self::Coptic | Self::Ethiopic | Self::EthiopicAmeteAlem => ce_days_in_month(y, m),
        })
    }

    /// The first day of `date`'s month.
    #[must_use]
    pub fn start_of_month(self, date: CalendarDate) -> CalendarDate {
        let f = self.fields(date);
        self.date(f.year, f.month, 1)
    }

    /// The last day of `date`'s month.
    #[must_use]
    pub fn end_of_month(self, date: CalendarDate) -> CalendarDate {
        let f = self.fields(date);
        self.date(f.year, f.month, self.days_in_month(f.year, f.month))
    }

    /// Whether `a` and `b` fall in the same month of the same year.
    #[must_use]
    pub fn same_month(self, a: CalendarDate, b: CalendarDate) -> bool {
        let (a, b) = (self.fields(a), self.fields(b));
        a.year == b.year && a.month == b.month
    }

    /// `n` months later (earlier when negative), the day cut to that month's length.
    #[must_use]
    pub fn add_months(self, date: CalendarDate, months: i64) -> CalendarDate {
        // More than CalendarDate's ten thousand years of 13 months moves nothing further.
        let months = months.clamp(-130_000, 130_000);
        let f = self.fields(date);
        let (mut year, mut month) = (f.year, s(i64::from(f.month)) + s(months));
        let in_year = |y: i32| s(i64::from(self.months_in_year(y)));
        while month > in_year(year) {
            month -= in_year(year);
            year = year.saturating_add(1);
        }
        while month < s(1) {
            year = year.saturating_sub(1);
            month += in_year(year);
        }
        let month = u8_of(month);
        self.date(year, month, f.day.min(self.days_in_month(year, month)))
    }

    /// `date` moved to `year`, keeping its month and day where that year has them
    /// (a Hebrew month after Adar keeps its name across a leap year).
    #[must_use]
    pub fn with_year(self, date: CalendarDate, year: i32) -> CalendarDate {
        let f = self.fields(date);
        let mut month = f.month;
        if self == Self::Hebrew && month > 6 {
            match (hebrew_leap(n(f.year)), hebrew_leap(n(year))) {
                (true, false) => month = month.saturating_sub(1),
                (false, true) => month = month.saturating_add(1),
                _ => {}
            }
        }
        let month = month.min(self.months_in_year(year));
        self.date(year, month, f.day.min(self.days_in_month(year, month)))
    }

    /// `date` moved to `month` of its year.
    #[must_use]
    pub fn with_month(self, date: CalendarDate, month: u8) -> CalendarDate {
        let f = self.fields(date);
        let month = month.max(1).min(self.months_in_year(f.year));
        self.date(f.year, month, f.day.min(self.days_in_month(f.year, month)))
    }

    /// The year a reader is shown for `date`, and its era.
    #[must_use]
    pub fn era(self, date: CalendarDate) -> (i32, Era) {
        let year = self.fields(date).year;
        match self {
            Self::Gregorian | Self::Hebrew => (year, Era::Unnamed),
            Self::Japanese => {
                let ymd = (date.year(), date.month(), date.day());
                let (_, before, era) = JAPANESE_ERAS
                    .iter()
                    .rev()
                    .find(|(start, _, _)| ymd >= *start)
                    .or(JAPANESE_ERAS.first())
                    .copied()
                    .unwrap_or(((1868, 9, 8), 1867, Era::Meiji));
                (year.saturating_sub(before), era)
            }
            Self::Buddhist => (year.saturating_add(543), Era::Be),
            Self::Taiwan if year > 1911 => (year.saturating_sub(1911), Era::Minguo),
            Self::Taiwan => (1912_i32.saturating_sub(year), Era::BeforeMinguo),
            Self::Persian => (year, Era::Ap),
            Self::Indian => (year, Era::Saka),
            Self::IslamicUmalqura | Self::IslamicCivil | Self::IslamicTabular => (year, Era::Ah),
            Self::Coptic if year <= 0 => (1_i32.saturating_sub(year), Era::Era0),
            Self::Ethiopic if year <= 0 => (year.saturating_add(5500), Era::Era0),
            Self::Coptic | Self::Ethiopic => (year, Era::Era1),
            Self::EthiopicAmeteAlem => (year, Era::Era0),
        }
    }
}

fn gregorian_leap(y: N) -> bool {
    rem(y, 4) == s(0) && (rem(y, 100) != s(0) || rem(y, 400) == s(0))
}

fn gregorian_days_in_month(y: N, m: N) -> N {
    n(match m.0 {
        2 if gregorian_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    })
}

fn gregorian_to_jd(y: N, m: N, d: N) -> N {
    let y1 = y - n(1);
    let offset = if m <= n(2) {
        n(0)
    } else if gregorian_leap(y) {
        n(-1)
    } else {
        n(-2)
    };
    n(GREGORIAN_EPOCH) - n(1) + n(365) * y1 + quot(y1, 4) - quot(y1, 100)
        + quot(y1, 400)
        + quot(n(367) * m - n(362), 12)
        + offset
        + d
}

fn persian_from_jd(jd: N) -> (N, N, N) {
    let since = jd - n(PERSIAN_EPOCH);
    let year = n(1) + quot(n(33) * since + n(3), 12_053);
    let farvardin1 = n(365) * (year - n(1)) + quot(n(8) * year + n(21), 33);
    let day_of_year = since - farvardin1;
    let month0 = if day_of_year < n(216) {
        quot(day_of_year, 31)
    } else {
        quot(day_of_year - n(6), 30)
    };
    let start = usize::try_from(month0.0)
        .ok()
        .and_then(|i| PERSIAN_MONTH_START.get(i))
        .copied()
        .unwrap_or_default();
    (year, month0 + n(1), day_of_year - n(start) + n(1))
}

fn persian_to_jd(y: N, m: N, d: N) -> N {
    let start = usize::try_from((m - n(1)).0)
        .ok()
        .and_then(|i| PERSIAN_MONTH_START.get(i))
        .copied()
        .unwrap_or_default();
    n(PERSIAN_EPOCH) - n(1) + n(365) * (y - n(1)) + quot(n(8) * y + n(21), 33) + n(start) + d
}

fn persian_days_in_month(y: N, m: N) -> N {
    if m <= n(6) {
        n(31)
    } else if m <= n(11) || rem(n(25) * y + n(11), 33) < n(8) {
        n(30)
    } else {
        n(29)
    }
}

fn indian_from_jd(jd: N) -> (N, N, N) {
    let gregorian = CalendarDate::from_days((jd - n(UNIX_JD)).0);
    let gy = n(gregorian.year());
    let mut year = gy - n(INDIAN_ERA_START);
    let mut year_day = jd - gregorian_to_jd(gy, n(1), n(1));
    let leap_month;
    if year_day < n(INDIAN_YEAR_START) {
        year -= n(1);
        leap_month = if gregorian_leap(gy - n(1)) { n(31) } else { n(30) };
        year_day += leap_month + n(255);
    } else {
        leap_month = if gregorian_leap(gy) { n(31) } else { n(30) };
        year_day -= n(INDIAN_YEAR_START);
    }
    if year_day < leap_month {
        return (year, n(1), year_day + n(1));
    }
    let month_day = year_day - leap_month;
    if month_day < n(155) {
        (year, quot(month_day, 31) + n(2), rem(month_day, 31) + n(1))
    } else {
        let month_day = month_day - n(155);
        (year, quot(month_day, 30) + n(7), rem(month_day, 30) + n(1))
    }
}

fn indian_to_jd(y: N, m: N, d: N) -> N {
    let gy = y + n(INDIAN_ERA_START);
    let (leap_month, mut jd) = if gregorian_leap(gy) {
        (n(31), gregorian_to_jd(gy, n(3), n(21)))
    } else {
        (n(30), gregorian_to_jd(gy, n(3), n(22)))
    };
    if m == n(1) {
        return jd + d - n(1);
    }
    jd += leap_month + n((m - n(2)).0.min(5)) * n(31);
    if m >= n(8) {
        jd += (m - n(7)) * n(30);
    }
    jd + d - n(1)
}

fn indian_days_in_month(y: N, m: N) -> N {
    if (m == n(1) && gregorian_leap(y + n(INDIAN_ERA_START))) || (n(2)..=n(6)).contains(&m) {
        n(31)
    } else {
        n(30)
    }
}

fn islamic_to_jd(epoch: i64, y: N, m: N, d: N) -> N {
    // ceil(29.5 * (m - 1))
    d + quot(n(59) * (m - n(1)) + n(1), 2) + (y - n(1)) * n(354) + quot(n(3) + n(11) * y, 30) + n(epoch) - n(1)
}

fn islamic_from_jd(epoch: i64, jd: N) -> (N, N, N) {
    let year = quot(n(30) * (jd - n(epoch)) + n(10_646), 10_631);
    // ceil(x / 29.5) + 1, at most 12
    let x = jd - (n(29) + islamic_to_jd(epoch, year, n(1), n(1)));
    let month = (quot(n(2) * x + n(58), 59) + n(1)).min(n(12));
    (year, month, jd - islamic_to_jd(epoch, year, month, n(1)) + n(1))
}

fn islamic_days_in_month(y: N, m: N) -> N {
    let leap = rem(n(14) + n(11) * y, 30) < n(11);
    n(29) + rem(m, 2) + n(i64::from(m == n(12) && leap))
}

fn umalqura_covers(y: N) -> bool {
    (n(UMALQURA_FIRST_YEAR)..=n(UMALQURA_LAST_YEAR)).contains(&y)
}

fn umalqura_bits(y: N) -> u16 {
    usize::try_from((y - n(UMALQURA_FIRST_YEAR)).0)
        .ok()
        .and_then(|i| UMALQURA_MONTHS.get(i))
        .copied()
        .unwrap_or_default()
}

fn umalqura_month_length(y: N, m: N) -> N {
    let shift = u32::try_from((n(12) - m).0.clamp(0, 11)).unwrap_or(0);
    let long = umalqura_bits(y).checked_shr(shift).unwrap_or(0) & 1 == 1;
    n(if long { 30 } else { 29 })
}

/// Days from the civil epoch to the first day of `y` (inside the table).
fn umalqura_year_start(y: N) -> N {
    let mut days = n(UMALQURA_START_DAYS);
    let mut year = n(UMALQURA_FIRST_YEAR);
    while year < y {
        days += n(348) + n(umalqura_bits(year).count_ones());
        year += n(1);
    }
    days
}

fn umalqura_month_start(y: N, m: N) -> N {
    let mut days = umalqura_year_start(y);
    let mut month = n(1);
    while month < m {
        days += umalqura_month_length(y, month);
        month += n(1);
    }
    days
}

fn umalqura_to_jd(y: N, m: N, d: N) -> N {
    if !umalqura_covers(y) {
        return islamic_to_jd(ISLAMIC_CIVIL_EPOCH, y, m, d);
    }
    n(ISLAMIC_CIVIL_EPOCH) + umalqura_month_start(y, m) + d - n(1)
}

fn umalqura_from_jd(jd: N) -> (N, N, N) {
    let days = jd - n(ISLAMIC_CIVIL_EPOCH);
    let first = umalqura_year_start(n(UMALQURA_FIRST_YEAR));
    let last = umalqura_year_start(n(UMALQURA_LAST_YEAR));
    if days < first || days > last {
        return islamic_from_jd(ISLAMIC_CIVIL_EPOCH, jd);
    }
    let mut year = n(UMALQURA_FIRST_YEAR);
    let mut start = first;
    loop {
        let next = start + n(348) + n(umalqura_bits(year).count_ones());
        if next > days || year >= n(UMALQURA_LAST_YEAR) {
            break;
        }
        start = next;
        year += n(1);
    }
    let mut month = n(1);
    let mut month_start = start;
    while month < n(12) && month_start + umalqura_month_length(year, month) <= days {
        month_start += umalqura_month_length(year, month);
        month += n(1);
    }
    (year, month, days - month_start + n(1))
}

fn hebrew_leap(y: N) -> bool {
    rem(y * n(7) + n(1), 19) < n(7)
}

fn hebrew_delay1(y: N) -> N {
    let months = quot(n(235) * y - n(234), 19);
    let parts = n(12_084) + n(13_753) * months;
    let day = months * n(29) + quot(parts, 25_920);
    if rem(n(3) * (day + n(1)), 7) < n(3) {
        day + n(1)
    } else {
        day
    }
}

fn hebrew_delay2(y: N) -> N {
    let (last, present, next) = (hebrew_delay1(y - n(1)), hebrew_delay1(y), hebrew_delay1(y + n(1)));
    if next - present == n(356) {
        n(2)
    } else if present - last == n(382) {
        n(1)
    } else {
        n(0)
    }
}

fn hebrew_start_of_year(y: N) -> N {
    hebrew_delay1(y) + hebrew_delay2(y)
}

fn hebrew_days_in_year(y: N) -> N {
    hebrew_start_of_year(y + n(1)) - hebrew_start_of_year(y)
}

fn hebrew_days_in_month(y: N, m: N) -> N {
    let leap = hebrew_leap(y);
    // Counted as in a leap year: a common year has no month 6 (Adar I).
    let m = if m >= n(6) && !leap { m + n(1) } else { m };
    let mut length = hebrew_days_in_year(y);
    if length > n(380) {
        length -= n(30);
    }
    n(match m.0 {
        4 | 7 | 9 | 11 | 13 => 29,
        2 if length == n(355) => 30,
        2 => 29,
        3 if length == n(353) => 29,
        3 => 30,
        6 if !leap => 0,
        _ => 30,
    })
}

fn hebrew_to_jd(y: N, m: N, d: N) -> N {
    let mut jd = hebrew_start_of_year(y);
    let mut month = n(1);
    while month < m {
        jd += hebrew_days_in_month(y, month);
        month += n(1);
    }
    jd + d + n(HEBREW_EPOCH)
}

fn hebrew_from_jd(jd: N) -> (N, N, N) {
    let d = jd - n(HEBREW_EPOCH);
    // Months since the epoch, rounded down: the year it gives may be one
    // short, which the loops below put right.
    let months = quot(d * n(HEBREW_DAY_PARTS), HEBREW_MONTH_PARTS);
    let mut year = quot(n(19) * months + n(234), 235) + n(1);
    let mut day_of_year = d - hebrew_start_of_year(year);
    while day_of_year < n(1) {
        year -= n(1);
        day_of_year = d - hebrew_start_of_year(year);
    }
    while day_of_year > hebrew_days_in_year(year) {
        day_of_year -= hebrew_days_in_year(year);
        year += n(1);
    }
    let mut month = n(1);
    let mut month_start = n(0);
    while month < n(13) && month_start + hebrew_days_in_month(year, month) < day_of_year {
        month_start += hebrew_days_in_month(year, month);
        month += n(1);
    }
    (year, month, day_of_year - month_start)
}

fn ce_to_jd(epoch: i64, y: N, m: N, d: N) -> N {
    n(epoch) + n(365) * y + quot(y, 4) + n(30) * (m - n(1)) + d - n(1)
}

fn ce_from_jd(epoch: i64, jd: N) -> (N, N, N) {
    let mut year = quot(n(4) * (jd - n(epoch)), 1461);
    if jd >= ce_to_jd(epoch, year + n(1), n(1), n(1)) {
        year += n(1);
    }
    let month = n(1) + quot(jd - ce_to_jd(epoch, year, n(1), n(1)), 30);
    (year, month, jd + n(1) - ce_to_jd(epoch, year, month, n(1)))
}

fn ce_days_in_month(y: N, m: N) -> N {
    if rem(m, 13) == n(0) {
        n(5) + quot(rem(y, 4), 3)
    } else {
        n(30)
    }
}
