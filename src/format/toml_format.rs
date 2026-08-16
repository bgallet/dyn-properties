use serde::de::Error as _;

use crate::Format;

/// Parses config files as TOML. Requires the `toml` Cargo feature.
pub struct Toml;

impl Format for Toml {
    type Error = toml::de::Error;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        let text = std::str::from_utf8(bytes).map_err(toml::de::Error::custom)?;
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Validate, DynProperties};
    use crate as dyn_properties;

    #[derive(DynProperties)]
    struct Cfg {
        #[range(min = 1, max = 100)]
        #[default(10)]
        count: u32,
    }

    #[test]
    fn parses_toml_bytes() {
        let cfg: Cfg = Toml::parse(b"count = 42").unwrap();
        assert_eq!(cfg.count, 42);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_toml() {
        let result: Result<Cfg, _> = Toml::parse(b"not valid = [");
        assert!(result.is_err());
    }
}
