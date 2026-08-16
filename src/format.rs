/// Abstracts "parse bytes into `T`" so [`PropertyWatcher`](crate::PropertyWatcher) isn't
/// tied to one config file format. Implementations are typically zero-sized marker types
/// selected at the type level — e.g. this crate's own [`Toml`](crate::Toml) — and the
/// derive macro's generated `serde::Deserialize` impl works with any of them unchanged,
/// since it's already plain, format-agnostic `serde`.
///
/// Implement this for your own format (YAML, RON, ...) to use it with `PropertyWatcher`
/// without needing a change to this crate.
pub trait Format {
    /// The error type produced when `bytes` isn't valid for this format, or doesn't match
    /// `T`'s shape.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parses `bytes` into `T`.
    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error>;
}

#[cfg(feature = "toml")]
mod toml_format;
#[cfg(feature = "toml")]
pub use toml_format::Toml;
