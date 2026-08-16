use std::fmt;
use std::time::Duration;

use serde::Deserialize;

/// Parses a compact duration string like `"30s"` into a [`std::time::Duration`].
///
/// [`std::time::Duration`] has no [`FromStr`](std::str::FromStr) impl of its own, and TOML
/// represents durations as this compact string rather than a table or a raw seconds count,
/// so any field typed `std::time::Duration` and bounded with
/// `#[range(min = "...", max = "...")]` (min/max as duration strings) routes through this
/// parser — the derive macro wires it in automatically, both for `#[default("...")]`
/// literals and for deserializing the field itself, so callers don't need to call it
/// directly except to parse a duration string outside of a `#[derive(DynProperties)]`
/// struct.
///
/// String grammar: `<digits><unit>`, where `<digits>` is one or more ASCII digits and
/// `<unit>` is one of `ms` (milliseconds), `s` (seconds), `m` (minutes), `h` (hours), or
/// `d` (days). No sign, decimal point, or whitespace is allowed (e.g. `"100ms"`, `"30s"`,
/// `"5m"`, `"2h"`, `"1d"`).
pub fn parse_duration(s: &str) -> Result<Duration, ParseDurationError> {
    let unit_start = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (digits, unit) = s.split_at(unit_start);
    if digits.is_empty() {
        return Err(ParseDurationError::NoDigits(s.to_string()));
    }
    if unit.is_empty() {
        return Err(ParseDurationError::NoUnit(s.to_string()));
    }
    let value: u64 = digits
        .parse()
        .map_err(|_| ParseDurationError::NumberTooLarge(s.to_string()))?;
    match unit {
        "ms" => Ok(Duration::from_millis(value)),
        "s" => Ok(Duration::from_secs(value)),
        "m" => Ok(Duration::from_secs(value * 60)),
        "h" => Ok(Duration::from_secs(value * 3600)),
        "d" => Ok(Duration::from_secs(value * 86400)),
        _ => Err(ParseDurationError::InvalidUnit {
            unit: unit.to_string(),
            input: s.to_string(),
        }),
    }
}

/// The string failed to parse as a duration: it wasn't `<digits>` followed by one of
/// `ms`, `s`, `m`, `h`, `d`.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseDurationError {
    NoDigits(String),
    NoUnit(String),
    InvalidUnit { unit: String, input: String },
    NumberTooLarge(String),
}

impl fmt::Display for ParseDurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDigits(input) => write!(f, "no digits in {input:?}"),
            Self::NoUnit(input) => write!(f, "no unit in {input:?}"),
            Self::InvalidUnit { unit, input } => write!(f, "invalid unit {unit:?} in {input:?}"),
            Self::NumberTooLarge(input) => write!(f, "number too large in {input:?}"),
        }
    }
}

impl std::error::Error for ParseDurationError {}

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
    }

    #[test]
    fn error_message_for_missing_digits() {
        assert_eq!(
            parse_duration("s").unwrap_err().to_string(),
            "no digits in \"s\""
        );
    }

    #[test]
    fn error_message_for_negative_values() {
        assert_eq!(
            parse_duration("-5s").unwrap_err().to_string(),
            "no digits in \"-5s\""
        );
    }

    #[test]
    fn error_message_for_empty_string() {
        assert_eq!(
            parse_duration("").unwrap_err().to_string(),
            "no digits in \"\""
        );
    }

    #[test]
    fn error_message_for_missing_unit() {
        assert_eq!(
            parse_duration("30").unwrap_err().to_string(),
            "no unit in \"30\""
        );
    }

    #[test]
    fn error_message_for_invalid_unit() {
        assert_eq!(
            parse_duration("100xyz").unwrap_err().to_string(),
            "invalid unit \"xyz\" in \"100xyz\""
        );
    }

    #[test]
    fn error_message_for_number_too_large() {
        assert_eq!(
            parse_duration("99999999999999999999s")
                .unwrap_err()
                .to_string(),
            "number too large in \"99999999999999999999s\""
        );
    }
}
