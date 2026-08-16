use crate::Format;

/// Parses config files as JSON. Requires the `json` Cargo feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Json;

impl Format for Json {
    type Error = serde_json::Error;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        serde_json::from_slice(bytes)
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
    fn parses_json_bytes() {
        let cfg: Cfg = Json::parse(br#"{"count": 42}"#).unwrap();
        assert_eq!(cfg.count, 42);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_json() {
        let result: Result<Cfg, _> = Json::parse(b"not valid json");
        assert!(result.is_err());
    }
}
