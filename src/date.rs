//! Parsing the `--date` spec that selects which day a journal entry is for.
//!
//! Journal entries are addressed by date, so backfilling yesterday or reopening a past day needs a way to name one. The
//! accepted forms are deliberately small: an absolute `YYYY-MM-DD`, the everyday names `today`/`yesterday`/`tomorrow`,
//! and a signed day offset like `-1` or `+3`.

use anyhow::{Context as _, Result, bail};
use chrono::{Days, NaiveDate};

/// Resolve a date spec against `today`.
///
/// `today` is passed in rather than read from the clock so the caller decides what "now" means (and so this is
/// testable). The sign on an offset is required, which keeps a bare number from being read as half a date.
pub fn parse(spec: &str, today: NaiveDate) -> Result<NaiveDate> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("date cannot be empty");
    }

    let lower = spec.to_lowercase();
    let named = match lower.as_str() {
        "today" => Some(0),
        "yesterday" => Some(-1),
        "tomorrow" => Some(1),
        _ => None,
    };

    if let Some(offset) = named.or_else(|| offset_days(&lower)) {
        return shift(today, offset).with_context(|| format!("date `{spec}` is out of range"));
    }

    NaiveDate::parse_from_str(spec, "%Y-%m-%d").with_context(|| {
        format!("invalid date `{spec}`; expected YYYY-MM-DD, today/yesterday/tomorrow, or an offset like -1")
    })
}

/// A signed day offset such as `-1` or `+3`. The sign is required, so `2026` stays a (bad) date rather than becoming an
/// offset of two thousand days.
fn offset_days(spec: &str) -> Option<i64> {
    if !spec.starts_with(['-', '+']) {
        return None;
    }

    spec.parse().ok()
}

/// Move `date` by `offset` days, or `None` when the result leaves the representable range.
const fn shift(date: NaiveDate, offset: i64) -> Option<NaiveDate> {
    let days = Days::new(offset.unsigned_abs());

    if offset < 0 {
        date.checked_sub_days(days)
    } else {
        date.checked_add_days(days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed "today" so every case is deterministic.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 27).unwrap()
    }

    fn parsed(spec: &str) -> NaiveDate {
        parse(spec, today()).unwrap()
    }

    fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn parses_absolute_dates() {
        assert_eq!(parsed("2026-01-05"), ymd(2026, 1, 5));
        // Surrounding whitespace is tolerated, as a shell can easily introduce it.
        assert_eq!(parsed("  2026-01-05  "), ymd(2026, 1, 5));
    }

    #[test]
    fn parses_named_days_case_insensitively() {
        assert_eq!(parsed("today"), today());
        assert_eq!(parsed("yesterday"), ymd(2026, 7, 26));
        assert_eq!(parsed("Tomorrow"), ymd(2026, 7, 28));
    }

    #[test]
    fn parses_signed_offsets() {
        assert_eq!(parsed("-1"), ymd(2026, 7, 26));
        assert_eq!(parsed("+3"), ymd(2026, 7, 30));
        // An offset crosses month and year boundaries like any other date arithmetic.
        assert_eq!(parsed("-27"), ymd(2026, 6, 30));
        assert_eq!(parsed("-208"), ymd(2025, 12, 31));
        // `-0` is just today.
        assert_eq!(parsed("-0"), today());
    }

    #[test]
    fn rejects_unsigned_numbers_and_malformed_dates() {
        // Without a sign a number is not an offset, so it falls through to the date parser and fails.
        for spec in ["3", "2026", "", "   ", "2026-13-01", "2026-02-30", "last tuesday", "-"] {
            assert!(parse(spec, today()).is_err(), "expected `{spec}` to be rejected");
        }
    }

    #[test]
    fn rejects_offsets_that_leave_the_calendar() {
        assert!(parse("+1", NaiveDate::MAX).is_err());
        assert!(parse("-1", NaiveDate::MIN).is_err());
    }
}
