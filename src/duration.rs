//! Parses human-friendly duration strings like `"30s"` or `"1h30m"` into a
//! [`std::time::Duration`].
//!
//! Parsing is delegated to and re-exported from the [`humantime`] crate.
//! It accepts a concatenation of `<number><unit>` spans, e.g. `"100ms"`, `"5m"`, `"2h 37min"`,
//! `"1h30m"`.
//! Supported units:
//! `ns`, `us`/`µs`, `ms`, `s`/`sec`/`seconds`, `m`/`min`/`minutes`, `h`/`hours`,
//! `d`/`days`, `w`/`weeks`, `M`/`months` (30.44 days), `y`/`years` (365.25 days).
//!
//! Warning that units are case-sensitive: `m` is minutes, `M` is months.
pub use humantime::{DurationError, parse_duration};

use serde::Deserialize;

use std::time::Duration;

/// Deserializes a `std::time::Duration`-typed field from its compact string form,
/// returning `Option<Duration>` to match the internal helper-struct representation the
/// derive macro uses for every bounded/defaulted field (including ones whose declared
/// type is already `Option<Duration>` — see the design doc's default-overlay mechanism).
/// Not meant to be called directly; wired in by the derive macro via
/// `#[serde(deserialize_with = "...")]` on `Duration`-kind fields.
#[doc(hidden)]
pub fn deserialize_duration_option<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<String> = Option::deserialize(deserializer)?;
    match value {
        Some(s) => parse_duration(&s)
            .map(Some)
            .map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_milliseconds() {
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
    }

    #[test]
    fn parses_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn parses_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn parses_hours() {
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn parses_days() {
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn parses_compound_spans() {
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration("2h 37min").unwrap(),
            Duration::from_secs(9420)
        );
    }

    #[test]
    fn rejects_invalid_suffix() {
        assert!(parse_duration("100xyz").is_err());
    }

    #[test]
    fn rejects_missing_digits() {
        assert!(parse_duration("s").is_err());
    }

    #[test]
    fn rejects_negative_values() {
        assert!(parse_duration("-5s").is_err());
    }

    #[test]
    fn rejects_empty_string() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration(" ").is_err());
    }

    #[test]
    fn rejects_missing_unit() {
        assert!(parse_duration("30").is_err());
    }

    #[test]
    fn rejects_overflow_instead_of_panicking_or_wrapping() {
        // Regression guard for the old hand-rolled parser, where the unit multiplication
        // (`value * 60` etc.) overflowed u64: debug builds panicked, release wrapped silently.
        assert!(parse_duration("18446744073709551615m").is_err());
        assert!(parse_duration("214011222337450d").is_err());
        assert!(parse_duration("99999999999999999999s").is_err());
    }

    #[test]
    fn month_and_year_units_are_case_sensitive() {
        assert_eq!(
            parse_duration("5M").unwrap(),
            Duration::from_secs(13_150_080)
        );
        assert_ne!(parse_duration("5m").unwrap(), parse_duration("5M").unwrap());
        assert_eq!(
            parse_duration("1y").unwrap(),
            Duration::from_secs(31_557_600)
        );
    }

    #[cfg(feature = "json")]
    #[test]
    fn deserialize_option_parses_strings_and_passes_none_through() {
        #[derive(Deserialize)]
        struct Wrapper {
            // Same pairing the derive macro's generated helper always uses.
            #[serde(default, deserialize_with = "deserialize_duration_option")]
            field: Option<Duration>,
        }

        let w: Wrapper = serde_json::from_str(r#"{"field": "90s"}"#).unwrap();
        assert_eq!(w.field, Some(Duration::from_secs(90)));

        let w: Wrapper = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(w.field, None);

        let w: Wrapper = serde_json::from_str(r#"{"field": null}"#).unwrap();
        assert_eq!(w.field, None);

        let result: Result<Wrapper, _> = serde_json::from_str(r#"{"field": "bogus"}"#);
        assert!(result.is_err());
    }
}
