/// Validates that `s` has valid duration-literal syntax (`<digits><unit>`, `<unit>` one of
/// `ms`/`s`/`m`/`h`/`d`) without constructing a value — a compile-time syntax check only,
/// mirroring `dyn_properties::parse_duration`'s grammar exactly.
///
/// Kept as a small, self-contained duplicate rather than a shared dependency:
/// `dyn-properties-derive` cannot depend on `dyn-properties` (that would be circular,
/// since `dyn-properties` depends on this crate to re-export the derive macro), and the
/// grammar is small and stable. If `dyn_properties::parse_duration`'s grammar
/// (`src/duration.rs` in the `dyn-properties` crate) ever changes, this must change with
/// it — both have unit tests covering the same cases.
pub fn validate_duration_literal_syntax(s: &str) -> Result<(), String> {
    let unit_start = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (digits, unit) = s.split_at(unit_start);
    if digits.is_empty() {
        return Err(format!("no digits in {s:?}"));
    }
    if unit.is_empty() {
        return Err(format!("no unit in {s:?}"));
    }
    if digits.parse::<u64>().is_err() {
        return Err(format!("number too large in {s:?}"));
    }
    match unit {
        "ms" | "s" | "m" | "h" | "d" => Ok(()),
        _ => Err(format!("invalid unit {unit:?} in {s:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_milliseconds() {
        assert!(validate_duration_literal_syntax("100ms").is_ok());
    }

    #[test]
    fn accepts_seconds() {
        assert!(validate_duration_literal_syntax("30s").is_ok());
    }

    #[test]
    fn accepts_minutes() {
        assert!(validate_duration_literal_syntax("5m").is_ok());
    }

    #[test]
    fn accepts_hours() {
        assert!(validate_duration_literal_syntax("2h").is_ok());
    }

    #[test]
    fn accepts_days() {
        assert!(validate_duration_literal_syntax("1d").is_ok());
    }

    #[test]
    fn rejects_invalid_suffix() {
        assert!(validate_duration_literal_syntax("100xyz").is_err());
    }

    #[test]
    fn rejects_missing_digits() {
        assert!(validate_duration_literal_syntax("s").is_err());
    }

    #[test]
    fn rejects_negative_values() {
        assert!(validate_duration_literal_syntax("-5s").is_err());
    }

    #[test]
    fn rejects_empty_string() {
        assert!(validate_duration_literal_syntax("").is_err());
    }

    #[test]
    fn error_message_for_missing_digits() {
        assert_eq!(
            validate_duration_literal_syntax("s").unwrap_err(),
            "no digits in \"s\""
        );
    }

    #[test]
    fn error_message_for_negative_values() {
        assert_eq!(
            validate_duration_literal_syntax("-5s").unwrap_err(),
            "no digits in \"-5s\""
        );
    }

    #[test]
    fn error_message_for_empty_string() {
        assert_eq!(
            validate_duration_literal_syntax("").unwrap_err(),
            "no digits in \"\""
        );
    }

    #[test]
    fn error_message_for_missing_unit() {
        assert_eq!(
            validate_duration_literal_syntax("30").unwrap_err(),
            "no unit in \"30\""
        );
    }

    #[test]
    fn error_message_for_invalid_unit() {
        assert_eq!(
            validate_duration_literal_syntax("100xyz").unwrap_err(),
            "invalid unit \"xyz\" in \"100xyz\""
        );
    }

    #[test]
    fn error_message_for_number_too_large() {
        assert_eq!(
            validate_duration_literal_syntax("99999999999999999999s").unwrap_err(),
            "number too large in \"99999999999999999999s\""
        );
    }
}
