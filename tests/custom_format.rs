use dyn_properties::{DynProperties, Format, PropertyWatcher};
use serde::de::value::{Error as ValueError, MapDeserializer};
use std::io::Write;
use std::time::Duration;

/// A trivial caller-defined format, proving `Format` is genuinely pluggable: not TOML or
/// JSON, no Cargo feature gate, defined entirely in this test file using only `serde`'s
/// own `de::value` helpers (part of the base `serde` crate — no extra dependency, and
/// this file has no `required-features` entry in Cargo.toml, so it runs in every feature
/// combination, including with neither `toml` nor `json` enabled).
///
/// "Parses" a bare decimal number into `{ count: <that number> }`.
struct PlainNumber;

/// A single-value deserializer for the `"count"` entry fed into
/// [`MapDeserializer`], used in place of `u64`'s own `IntoDeserializer` impl.
///
/// `serde`'s built-in primitive deserializers (the ones `u64: IntoDeserializer`
/// hands out) forward `deserialize_option` to `deserialize_any` — see
/// `forward_to_deserialize_any!` on e.g. `U64Deserializer` in
/// `serde::de::value`. That's fine for a struct field typed as a bare `u64`,
/// but this crate's derive macro generates a deserialization helper struct
/// where every field is `#[serde(default)] Option<FieldType>` (so it can tell
/// "absent, use the default" apart from "present"). `Option<T>::deserialize`
/// calls `deserializer.deserialize_option(..)`, and a deserializer that just
/// forwards that to `deserialize_any` ends up calling `Visitor::visit_u64` on
/// `Option`'s own visitor — which only implements `visit_some`/`visit_none` —
/// producing "invalid type: integer `42`, expected option" instead of `Some(42)`.
/// Implementing `deserialize_option` explicitly (`visitor.visit_some(self)`)
/// fixes that.
#[derive(Clone, Copy)]
struct CountValue(u64);

impl<'de> serde::de::IntoDeserializer<'de, ValueError> for CountValue {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> serde::de::Deserializer<'de> for CountValue {
    type Error = ValueError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_u64(self.0)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

impl Format for PlainNumber {
    type Error = ValueError;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        let text = std::str::from_utf8(bytes).map_err(|e| serde::de::Error::custom(e.to_string()))?;
        let value: u64 = text
            .trim()
            .parse()
            .map_err(|_| serde::de::Error::custom(format!("not a plain decimal number: {text}")))?;
        let pairs = vec![("count", CountValue(value))];
        T::deserialize(MapDeserializer::new(pairs.into_iter()))
    }
}

#[derive(DynProperties)]
struct Cfg {
    #[range(min = 0, max = 1000)]
    #[default(0)]
    count: u32,
}

#[tokio::test]
async fn watcher_works_with_a_caller_defined_format() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "42").unwrap();

    let watcher = PropertyWatcher::<Cfg, PlainNumber>::start(file.path(), Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(watcher.load().count, 42);
}
