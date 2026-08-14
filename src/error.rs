use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    TomlParse(toml::de::Error),
    Validation { field_path: String, reason: String },
}

impl Error {
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
            Error::TomlParse(e) => write!(f, "TOML parse error: {e}"),
            Error::Validation { field_path, reason } => write!(f, "{field_path}: {reason}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::TomlParse(e) => Some(e),
            Error::Validation { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::TomlParse(e)
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
        let parse_err = toml::from_str::<toml::Value>("not valid = [").unwrap_err();
        let err = Error::TomlParse(parse_err);
        let prefixed = err.prefixed("pool");
        assert!(matches!(prefixed, Error::TomlParse(_)));
    }
}
