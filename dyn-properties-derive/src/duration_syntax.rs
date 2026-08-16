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
    let err = || {
        format!("invalid duration string `{s}`: expected digits followed by one of ms, s, m, h, d")
    };
    let unit_start = s.find(|c: char| !c.is_ascii_digit()).ok_or_else(err)?;
    let (digits, unit) = s.split_at(unit_start);
    if digits.is_empty() || digits.parse::<u64>().is_err() {
        return Err(err());
    }
    match unit {
        "ms" | "s" | "m" | "h" | "d" => Ok(()),
        _ => Err(err()),
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
}
