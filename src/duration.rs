use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use std::time::Duration as StdDuration;

/// A [`std::time::Duration`] that (de)serializes from a compact string like `"30s"`
/// instead of TOML's native table/seconds representation, and that fields bounded with
/// `#[range(min = "...", max = "...")]` (min/max as duration strings) must use.
///
/// String grammar: `<digits><unit>`, where `<digits>` is one or more ASCII digits and
/// `<unit>` is one of `ms` (milliseconds), `s` (seconds), `m` (minutes), `h` (hours), or
/// `d` (days). No sign, decimal point, or whitespace is allowed (e.g. `"100ms"`, `"30s"`,
/// `"5m"`, `"2h"`, `"1d"`). Parsing is exposed via [`FromStr`] and via [`serde::Deserialize`].
///
/// Derefs to `std::time::Duration` for comparisons and other standard operations.
///
/// Deliberately does **not** implement [`Default`]: a silent zero-duration is rarely the
/// right fallback for a timeout or interval. Every plain `Duration` field on a
/// `#[derive(DynProperties)]` struct must carry an explicit `#[default("...")]` attribute
/// — the derive macro rejects one that doesn't, at compile time. If "unset" is a
/// meaningful state for a field, use `Option<Duration>` instead, which defaults to `None`
/// without needing an explicit `#[default(..)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(StdDuration);

impl Deref for Duration {
    type Target = StdDuration;
    fn deref(&self) -> &StdDuration {
        &self.0
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// The string failed to parse as a [`Duration`]: it wasn't `<digits>` followed by one of
/// `ms`, `s`, `m`, `h`, `d`.
#[derive(Debug, PartialEq, Eq)]
pub struct ParseDurationError(String);

impl fmt::Display for ParseDurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid duration string `{}`: expected digits followed by one of ms, s, m, h, d",
            self.0
        )
    }
}

impl std::error::Error for ParseDurationError {}

impl FromStr for Duration {
    type Err = ParseDurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let unit_start = s
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| ParseDurationError(s.to_string()))?;
        let (digits, unit) = s.split_at(unit_start);
        if digits.is_empty() {
            return Err(ParseDurationError(s.to_string()));
        }
        let value: u64 = digits.parse().map_err(|_| ParseDurationError(s.to_string()))?;
        let std_duration = match unit {
            "ms" => StdDuration::from_millis(value),
            "s" => StdDuration::from_secs(value),
            "m" => StdDuration::from_secs(value * 60),
            "h" => StdDuration::from_secs(value * 3600),
            "d" => StdDuration::from_secs(value * 86400),
            _ => return Err(ParseDurationError(s.to_string())),
        };
        Ok(Duration(std_duration))
    }
}

impl<'de> serde::Deserialize<'de> for Duration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_milliseconds() {
        assert_eq!("100ms".parse::<Duration>().unwrap().0, StdDuration::from_millis(100));
    }

    #[test]
    fn parses_seconds() {
        assert_eq!("30s".parse::<Duration>().unwrap().0, StdDuration::from_secs(30));
    }

    #[test]
    fn parses_minutes() {
        assert_eq!("5m".parse::<Duration>().unwrap().0, StdDuration::from_secs(300));
    }

    #[test]
    fn parses_hours() {
        assert_eq!("2h".parse::<Duration>().unwrap().0, StdDuration::from_secs(7200));
    }

    #[test]
    fn parses_days() {
        assert_eq!("1d".parse::<Duration>().unwrap().0, StdDuration::from_secs(86400));
    }

    #[test]
    fn rejects_invalid_suffix() {
        assert!("100xyz".parse::<Duration>().is_err());
    }

    #[test]
    fn rejects_missing_digits() {
        assert!("s".parse::<Duration>().is_err());
    }

    #[test]
    fn rejects_negative_values() {
        assert!("-5s".parse::<Duration>().is_err());
    }

    #[test]
    fn rejects_empty_string() {
        assert!("".parse::<Duration>().is_err());
    }
}
