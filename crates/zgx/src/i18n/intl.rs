//! `Intl`-style formatting for a locale: lists, numbers (with percent and
//! currency), dates and relative times, by ICU4X's compiled data. The
//! options are ECMA-402's (`Intl.ListFormat`, `NumberFormat`,
//! `DateTimeFormat`, `RelativeTimeFormat`) that apps ask for, and the output
//! follows what Chrome gives for them, up to the CLDR version the data is
//! from. Behind `i18n-intl`.
use icu_locale_core::Locale;
use std::fmt::Write;

fn locale_of(tag: &str) -> Locale {
    Locale::try_from_str(tag).unwrap_or(icu_locale_core::locale!("en"))
}

/// A list's kind (`Intl.ListFormat`'s `type`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListType {
    /// "a, b, and c".
    #[default]
    Conjunction,
    /// "a, b, or c".
    Disjunction,
    /// "a, b, c", as units are listed.
    Unit,
}
/// How long a list or relative time is written (`style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// Spelled out.
    #[default]
    Long,
    /// Abbreviated.
    Short,
    /// As short as can be.
    Narrow,
}

/// `items` as a list in `locale`.
pub fn format_list(locale: &str, items: &[String], kind: ListType, style: Style) -> String {
    use icu_list::{
        ListFormatter,
        options::{ListFormatterOptions, ListLength},
    };
    let options = ListFormatterOptions::default().with_length(match style {
        Style::Long => ListLength::Wide,
        Style::Short => ListLength::Short,
        Style::Narrow => ListLength::Narrow,
    });
    let prefs = (&locale_of(locale)).into();
    let formatter = match kind {
        ListType::Conjunction => ListFormatter::try_new_and(prefs, options),
        ListType::Disjunction => ListFormatter::try_new_or(prefs, options),
        ListType::Unit => ListFormatter::try_new_unit(prefs, options),
    };
    match formatter {
        Ok(formatter) => {
            use writeable::Writeable;
            formatter
                .format(items.iter().map(String::as_str))
                .write_to_string()
                .into_owned()
        }
        Err(_) => items.join(", "),
    }
}

/// What a number is written as (`style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NumberStyle {
    /// Plain.
    #[default]
    Decimal,
    /// A fraction as a percentage: 0.25 is "25%".
    Percent,
    /// An amount of `NumberOptions::currency`.
    Currency,
}
/// `Intl.NumberFormat`'s options.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NumberOptions {
    /// Decimal, percent or currency.
    pub style: NumberStyle,
    /// An ISO 4217 code, for `Currency`.
    pub currency: Option<String>,
    /// The fewest fraction digits.
    pub minimum_fraction_digits: Option<u8>,
    /// The most fraction digits.
    pub maximum_fraction_digits: Option<u8>,
    /// Whether to group digits (default: yes).
    pub use_grouping: Option<bool>,
}

/// The digits a currency has after its point (ISO 4217's minor unit).
fn currency_digits(code: &str) -> u8 {
    match code {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

/// `n` in `locale`.
pub fn format_number(locale: &str, n: f64, options: &NumberOptions) -> String {
    use fixed_decimal::{Decimal, FloatPrecision, SignedRoundingMode, UnsignedRoundingMode};
    let tag = locale_of(locale);
    let code = options
        .currency
        .as_deref()
        .unwrap_or("")
        .to_ascii_uppercase();
    let (default_min, default_max) = match options.style {
        NumberStyle::Decimal => (0, 3),
        NumberStyle::Percent => (0, 0),
        NumberStyle::Currency => (currency_digits(&code), currency_digits(&code)),
    };
    let max = options
        .maximum_fraction_digits
        .unwrap_or_else(|| default_max.max(options.minimum_fraction_digits.unwrap_or(0)));
    let min = options
        .minimum_fraction_digits
        .unwrap_or(default_min)
        .min(max);
    let Ok(mut value) = Decimal::try_from_f64(n, FloatPrecision::RoundTrip) else {
        return n.to_string();
    };
    // Rounded half away from zero, as ECMA-402 does.
    let shift = if options.style == NumberStyle::Percent {
        2
    } else {
        0
    };
    value.round_with_mode(
        -i16::from(max) - shift,
        SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
    );
    value.multiply_pow10(shift);
    value.trim_start();
    value.pad_start(1);
    value.pad_end(-i16::from(min));
    let grouping = if options.use_grouping == Some(false) {
        Some(icu_decimal::options::GroupingStrategy::Never)
    } else {
        None
    };
    let mut decimal_options = icu_decimal::options::DecimalFormatterOptions::default();
    decimal_options.grouping_strategy = grouping;
    let mut out = String::new();
    match options.style {
        NumberStyle::Percent => {
            use icu_experimental::dimension::percent::{
                formatter::PercentFormatter, options::PercentFormatterOptions,
            };
            let _ = decimal_options;
            if let Ok(formatter) =
                PercentFormatter::try_new((&tag).into(), PercentFormatterOptions::default())
            {
                use writeable::Writeable;
                let _ = formatter.format(&value).write_to(&mut out);
                return out;
            }
        }
        NumberStyle::Currency => {
            use icu_experimental::dimension::currency::{
                CurrencyType, formatter::CurrencyFormatter, options::CurrencyFormatterOptions,
            };
            if let Ok(currency) = code.parse::<CurrencyType>()
                && let Ok(formatter) = CurrencyFormatter::try_new_symbol(
                    (&tag).into(),
                    currency,
                    CurrencyFormatterOptions::default(),
                )
            {
                use writeable::Writeable;
                let _ = formatter.format_fixed_decimal(&value).write_to(&mut out);
                return out;
            }
        }
        NumberStyle::Decimal => {}
    }
    match icu_decimal::DecimalFormatter::try_new((&tag).into(), decimal_options) {
        Ok(formatter) => {
            let _ = write!(out, "{}", formatter.format(&value));
            out
        }
        Err(_) => value.to_string(),
    }
}

/// A relative time's unit (`RelativeTimeFormat`'s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Seconds.
    Second,
    /// Minutes.
    Minute,
    /// Hours.
    Hour,
    /// Days.
    Day,
    /// Weeks.
    Week,
    /// Months.
    Month,
    /// Quarters.
    Quarter,
    /// Years.
    Year,
}
impl Unit {
    /// The unit a name (`"day"` or `"days"`) says.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim_end_matches('s') {
            "second" => Unit::Second,
            "minute" => Unit::Minute,
            "hour" => Unit::Hour,
            "day" => Unit::Day,
            "week" => Unit::Week,
            "month" => Unit::Month,
            "quarter" => Unit::Quarter,
            "year" => Unit::Year,
            _ => return None,
        })
    }
}

/// `delta` of `unit`s from now ("in 2 hours", "3 months ago"); with
/// `numeric_auto`, "yesterday" and "tomorrow" where the locale has them.
pub fn format_relative(
    locale: &str,
    delta: f64,
    unit: Unit,
    numeric_auto: bool,
    style: Style,
) -> String {
    use fixed_decimal::{Decimal, FloatPrecision};
    use icu_experimental::relativetime::{
        RelativeTimeFormatter, RelativeTimeFormatterOptions, options::Numeric,
    };
    let prefs = (&locale_of(locale)).into();
    let mut options = RelativeTimeFormatterOptions::default();
    if numeric_auto {
        options = options.with_numeric(Numeric::Auto);
    }
    macro_rules! make {
        ($long:ident, $short:ident, $narrow:ident) => {
            match style {
                Style::Long => RelativeTimeFormatter::$long(prefs, options),
                Style::Short => RelativeTimeFormatter::$short(prefs, options),
                Style::Narrow => RelativeTimeFormatter::$narrow(prefs, options),
            }
        };
    }
    let formatter = match unit {
        Unit::Second => make!(
            try_new_long_second,
            try_new_short_second,
            try_new_narrow_second
        ),
        Unit::Minute => make!(
            try_new_long_minute,
            try_new_short_minute,
            try_new_narrow_minute
        ),
        Unit::Hour => make!(try_new_long_hour, try_new_short_hour, try_new_narrow_hour),
        Unit::Day => make!(try_new_long_day, try_new_short_day, try_new_narrow_day),
        Unit::Week => make!(try_new_long_week, try_new_short_week, try_new_narrow_week),
        Unit::Month => make!(
            try_new_long_month,
            try_new_short_month,
            try_new_narrow_month
        ),
        Unit::Quarter => make!(
            try_new_long_quarter,
            try_new_short_quarter,
            try_new_narrow_quarter
        ),
        Unit::Year => make!(try_new_long_year, try_new_short_year, try_new_narrow_year),
    };
    let (Ok(formatter), Ok(value)) = (
        formatter,
        Decimal::try_from_f64(delta, FloatPrecision::RoundTrip),
    ) else {
        return delta.to_string();
    };
    let mut out = String::new();
    use writeable::Writeable;
    let _ = formatter.format(value).write_to(&mut out);
    out
}

/// How much of a date or time one `dateStyle`/`timeStyle` shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateStyle {
    /// "Thursday, November 12, 2026".
    Full,
    /// "November 12, 2026".
    Long,
    /// "Nov 12, 2026".
    Medium,
    /// "11/12/26".
    Short,
}
/// How a date or time component is written (`month: "short"`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// "November", "Thursday".
    Long,
    /// "Nov", "Thu".
    Short,
    /// "N", "T".
    Narrow,
    /// "11", "12" (also `"2-digit"`).
    Numeric,
}
/// `Intl.DateTimeFormat`'s options, as far as they go: the styles, or the
/// components asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DateOptions {
    /// `dateStyle`.
    pub date_style: Option<DateStyle>,
    /// `timeStyle`.
    pub time_style: Option<DateStyle>,
    /// `weekday`.
    pub weekday: Option<Width>,
    /// `year`.
    pub year: Option<Width>,
    /// `month`.
    pub month: Option<Width>,
    /// `day`.
    pub day: Option<Width>,
    /// `hour`.
    pub hour: Option<Width>,
    /// `minute`.
    pub minute: Option<Width>,
    /// `second`.
    pub second: Option<Width>,
    /// `hour12`: `Some(true)` for 12-hour time, `Some(false)` for 24; unset,
    /// the locale's.
    pub hour12: Option<bool>,
}

/// The date and time `ms` milliseconds after 1970 began, `offset` seconds
/// east of UTC, as (year, month, day, hour, minute, second, nanosecond).
fn civil(ms: f64, offset: i64) -> (i32, u8, u8, u8, u8, u8, u32) {
    let whole = (ms / 1000.).floor();
    let nanos = ((ms - whole * 1000.) * 1e6).round() as u32;
    let seconds = whole as i64 + offset;
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (
        year,
        month,
        day,
        (rest / 3600) as u8,
        (rest / 60 % 60) as u8,
        (rest % 60) as u8,
        nanos.min(999_999_999),
    )
}

/// The time `ms` milliseconds after 1970 began in `locale`, `offset`
/// seconds east of UTC (`None`: this machine's zone).
pub fn format_date(locale: &str, ms: f64, offset: Option<i64>, options: &DateOptions) -> String {
    use icu_datetime::{
        DateTimeFormatter,
        fieldsets::builder::{DateFields, FieldSetBuilder},
        input::DateTime,
        options::{Length, TimePrecision},
    };
    let offset = offset.unwrap_or_else(|| i64::from(crate::local_time::offset()));
    let (year, month, day, hour, minute, second, nanos) = civil(ms, offset);
    let mut builder = FieldSetBuilder::new();
    let mut date_length = None;
    if options.date_style.is_some() || options.time_style.is_some() {
        match options.date_style {
            Some(DateStyle::Full) => {
                builder.date_fields = Some(DateFields::YMDE);
                date_length = Some(Length::Long);
            }
            Some(DateStyle::Long) => {
                builder.date_fields = Some(DateFields::YMD);
                date_length = Some(Length::Long);
            }
            Some(DateStyle::Medium) => {
                builder.date_fields = Some(DateFields::YMD);
                date_length = Some(Length::Medium);
            }
            Some(DateStyle::Short) => {
                builder.date_fields = Some(DateFields::YMD);
                date_length = Some(Length::Short);
            }
            None => {}
        }
        if let Some(time) = options.time_style {
            builder.time_precision = Some(match time {
                DateStyle::Short => TimePrecision::Minute,
                _ => TimePrecision::Second,
            });
            date_length.get_or_insert(match time {
                DateStyle::Short => Length::Short,
                _ => Length::Medium,
            });
        }
    } else {
        let any = |w: Option<Width>| w.is_some();
        let (y, m, d, e) = (
            any(options.year),
            any(options.month),
            any(options.day),
            any(options.weekday),
        );
        builder.date_fields = match (y, m, d, e) {
            (true, true, true, true) => Some(DateFields::YMDE),
            (true, true, true, false) => Some(DateFields::YMD),
            (false, true, true, true) => Some(DateFields::MDE),
            (false, true, true, false) => Some(DateFields::MD),
            (_, false, true, true) => Some(DateFields::DE),
            (false, false, true, false) => Some(DateFields::D),
            (false, false, false, true) => Some(DateFields::E),
            (false, true, false, false) => Some(DateFields::M),
            (true, true, false, _) => Some(DateFields::YM),
            (true, false, false, _) => Some(DateFields::Y),
            _ => None,
        };
        let widest = [options.month, options.weekday]
            .into_iter()
            .flatten()
            .min_by_key(|w| match w {
                Width::Long => 0,
                Width::Short | Width::Narrow => 1,
                Width::Numeric => 2,
            });
        date_length = Some(match widest {
            Some(Width::Long) => Length::Long,
            Some(Width::Short | Width::Narrow) => Length::Medium,
            Some(Width::Numeric) | None => {
                if options.month == Some(Width::Numeric) {
                    Length::Short
                } else {
                    Length::Medium
                }
            }
        });
        if options.hour.is_some() || options.minute.is_some() || options.second.is_some() {
            builder.time_precision = Some(if options.second.is_some() {
                TimePrecision::Second
            } else if options.minute.is_some() {
                TimePrecision::Minute
            } else {
                TimePrecision::Hour
            });
        }
    }
    builder.length = date_length;
    let tag = locale_of(locale);
    let iso = format!("{year:04}-{month:02}-{day:02}");
    let (Ok(date), Ok(time)) = (
        icu_calendar::Date::try_new_iso(year, month, day),
        icu_datetime::input::Time::try_new(hour, minute, second, nanos),
    ) else {
        return iso;
    };
    let prefs: icu_datetime::DateTimeFormatterPreferences = (&tag).into();
    let has_time = builder.time_precision.is_some();
    let period = matches!(
        builder.date_fields,
        Some(DateFields::M | DateFields::YM | DateFields::Y)
    );
    let formatted = match (builder.date_fields.is_some(), has_time) {
        (true, true) => builder.build_date_and_time().ok().and_then(|fields| {
            DateTimeFormatter::try_new(prefs, fields)
                .ok()
                .map(|f| f.format(&DateTime { date, time }).to_string())
        }),
        (true, false) if period => builder.build_calendar_period().ok().and_then(|fields| {
            DateTimeFormatter::try_new(prefs, fields)
                .ok()
                .map(|f| f.format(&date).to_string())
        }),
        (true, false) => builder.build_date().ok().and_then(|fields| {
            DateTimeFormatter::try_new(prefs, fields)
                .ok()
                .map(|f| f.format(&date).to_string())
        }),
        (false, true) => builder.build_time().ok().and_then(|fields| {
            DateTimeFormatter::try_new(prefs, fields)
                .ok()
                .map(|f| f.format(&time).to_string())
        }),
        (false, false) => None,
    };
    // V8 writes the narrow no-break space CLDR puts before AM/PM as a space.
    formatted.map_or(iso, |text| text.replace('\u{202f}', " "))
}
