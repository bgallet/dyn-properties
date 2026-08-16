use std::fmt;

/// Everything that can go wrong loading and validating a config file: reading it,
/// parsing it (in whichever [`Format`](crate::Format) was selected), or checking it
/// against `#[range]`/`#[len]` bounds.
#[derive(Debug)]
pub enum Error {
    /// The config file could not be read (e.g. it doesn't exist or isn't readable).
    Io(std::io::Error),
    /// The file's contents didn't parse under the selected format, or didn't match the
    /// target struct's shape.
    Parse(Box<dyn std::error::Error + Send + Sync>),
    /// The file parsed fine but a field violated its declared bound.
    Validation {
        /// Dot-separated path to the offending field, e.g. `"pool.idle_timeout"` for a
        /// nested struct.
        field_path: String,
        /// Human-readable description of why the value is out of bounds.
        reason: String,
    },
}

impl Error {
    /// Prepends `parent_field` to a [`Error::Validation`]'s `field_path`, turning e.g.
    /// `"idle_timeout"` into `"pool.idle_timeout"` when a nested struct's validation
    /// error bubbles up through its parent. Non-`Validation` variants pass through
    /// unchanged. Used by the derive macro's generated `Validate` impls for nested
    /// struct fields; not typically called directly.
    pub fn prefixed(self, parent_field: &str) -> Self {
        match self {
            Error::Validation { field_path, reason } => Error::Validation {
                field_path: format!("{parent_field}.{field_path}"),
                reason,
            },
            other => other,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Parse(e) => write!(f, "parse error: {e}"),
            Error::Validation { field_path, reason } => write!(f, "{field_path}: {reason}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Parse(e) => Some(e.as_ref()),
            Error::Validation { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_prepends_parent_field_to_validation_error() {
        let err = Error::Validation {
            field_path: "idle".to_string(),
            reason: "too big".to_string(),
        };
        let prefixed = err.prefixed("pool");
        match prefixed {
            Error::Validation { field_path, .. } => assert_eq!(field_path, "pool.idle"),
            _ => panic!("expected Validation variant"),
        }
    }

    #[test]
    fn prefixed_leaves_non_validation_variants_unchanged() {
        let parse_err = "abc".parse::<i32>().unwrap_err();
        let err = Error::Parse(Box::new(parse_err));
        let prefixed = err.prefixed("pool");
        assert!(matches!(prefixed, Error::Parse(_)));
    }
}
