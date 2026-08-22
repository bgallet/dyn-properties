#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Reads a config file — TOML, JSON, or your own [`Format`] — into a validated,
//! hot-reloadable struct.
//!
//! ```no_run
//! # #[cfg(feature = "toml")]
//! # {
//! use dyn_properties::{DynProperties, PropertyWatcher, Toml};
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[len(min = 3, max = 64)]
//!     #[default("localhost")]
//!     host: String,
//!
//!     #[range(min = 1, max = 65535)]
//!     #[default(8080)]
//!     port: u16,
//!
//!     #[range(min = "100ms", max = "30s")]
//!     #[default("5s")]
//!     request_timeout: Duration,
//! }
//!
//! # fn run() -> Result<(), dyn_properties::Error> {
//! let watcher = PropertyWatcher::<AppConfig, Toml>::start(
//!     "config.toml",
//!     Duration::from_secs(30),
//! )?;
//!
//! // Short-lived, same-thread read:
//! let port = watcher.load().port;
//!
//! // Passing the config to another thread or an async task: clone the Arc
//! // out first. arc-swap documents that Guards use a bounded pool of
//! // fast thread-local slots and aren't meant to be held long-term or
//! // moved across threads.
//! let cfg: Arc<AppConfig> = Arc::clone(&watcher.load());
//! some_other_fn(cfg);
//! # let _ = port;
//! # Ok(())
//! # }
//! # fn some_other_fn(_cfg: Arc<AppConfig>) {}
//! # }
//! ```
//!
//! ## Testing your config struct
//!
//! An out-of-bounds `#[default(..)]` is only caught by [`Validate`] the
//! first time it's actually constructed, rather than at compile time.
//! Add a test like this for any struct with defaults:
//!
//! ```
//! # use dyn_properties::{DynProperties, Validate};
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[range(min = 1, max = 65535)]
//!     #[default(8080)]
//!     port: u16,
//! }
//!
//! assert!(AppConfig::default().validate().is_ok());
//! ```
//!
//! ## Fields that don't fit String/numeric/Duration/nested
//!
//! A field whose type is `Vec<T>`, `HashMap<K, V>`, `BTreeMap`, `HashSet`, `BTreeSet`,
//! or `VecDeque` is recognized automatically: it's deserialized and defaulted like any
//! other field (falling back to an empty collection if `#[default(...)]` is omitted),
//! but isn't recursively validated, since a plain collection has no `Validate` impl of
//! its own to call. `#[default(...)]` on one of these takes any expression of the
//! field's own type — not just a literal:
//!
//! ```
//! # use dyn_properties::DynProperties;
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[default(vec!["dev".to_string()])]
//!     tags: Vec<String>,
//! }
//! ```
//!
//! Any *other* field type that isn't itself `#[derive(DynProperties)]` — a plain enum,
//! a third-party struct, a non-`std` map type — needs the same treatment but can't be
//! recognized by name; mark it `#[opaque]` explicitly:
//!
//! ```
//! # use dyn_properties::DynProperties;
//! #[derive(serde::Deserialize, Default)]
//! enum LogFormat {
//!     #[default]
//!     Text,
//!     Json,
//! }
//!
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[opaque]
//!     format: LogFormat,
//! }
//! ```
//!
//! Without `#[opaque]`, a field like this is assumed to be its own
//! `#[derive(DynProperties)]` struct and the generated code calls `Validate::validate`
//! on it — which fails to compile for a type that doesn't implement it.
//!
//! ## Required fields
//!
//! Some values — an API key, a database password — should never fall back to a
//! silently-defaulted value: shipping a dev-environment default to production is worse
//! than failing loudly. Mark a field `#[required]` instead of giving it a `#[default]`,
//! and loading fails with [`Error::Parse`] if it's absent from the file:
//!
//! ```
//! # #[cfg(feature = "toml")]
//! # {
//! use dyn_properties::{DynProperties, Format, Toml};
//!
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[required]
//!     api_key: String,
//! }
//!
//! assert!(Toml::parse::<AppConfig>(b"").is_err());
//! assert!(Toml::parse::<AppConfig>(br#"api_key = "abc123""#).is_ok());
//! # }
//! ```
//!
//! `#[required]` cannot be combined with `#[default(...)]` on the same field (they
//! contradict each other), and cannot be used on an `Option<T>` field (which already
//! means "absence is fine, gives `None`").
//!
//! A required field nested inside another `#[derive(DynProperties)]` struct
//! automatically enforces its own section's presence, even if the *outer* field isn't
//! itself marked `#[required]` — omitting the whole section is rejected exactly like
//! omitting the required field directly would be:
//!
//! ```
//! # #[cfg(feature = "toml")]
//! # {
//! use dyn_properties::{DynProperties, Format, Toml};
//!
//! #[derive(DynProperties)]
//! struct SecretConfig {
//!     #[required]
//!     password: String,
//! }
//!
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     secret: SecretConfig, // not itself #[required] — doesn't need to be
//! }
//!
//! // Omitting `[secret]` entirely is rejected, not just an empty `[secret]`.
//! assert!(Toml::parse::<AppConfig>(b"").is_err());
//! # }
//! ```
//!
//! This propagation only applies to a *bare* nested field — wrapping it in
//! `Option<SecretConfig>` is an explicit "this whole section is optional" signal that
//! wins over the inner `#[required]`, giving `None` when the section is absent rather
//! than an error.
//!
//! ## Cargo features
//!
//! Neither format is enabled by default — enable exactly the one(s) you need:
//!
//! - `toml` — adds [`Toml`], parsing config files as TOML.
//! - `json` — adds [`Json`], parsing config files as JSON.
//! - `tokio` — adds [`tokio::PropertyWatcher`], a tokio-native counterpart to the
//!   default thread-based [`PropertyWatcher`] (see "Tokio" below).
//!
//! The two format features can be enabled together. With neither enabled, [`Format`]
//! itself is still available — implement it for your own format (YAML, RON, ...) and
//! use `PropertyWatcher<T, YourFormat>` without depending on `toml` or `serde_json` at
//! all.
//!
//! ## Tokio
//!
//! [`PropertyWatcher`] always works: it polls its file from a dedicated `std::thread`,
//! no async runtime required. If you're already running a tokio runtime, enable the
//! `tokio` feature and use [`tokio::PropertyWatcher`] instead — it spawns no extra OS
//! thread (refresh runs as a `tokio::spawn`'d task on your own runtime) and its
//! `subscribe()` returns a native `tokio::sync::watch::Receiver`. If you start the
//! thread-based [`PropertyWatcher`] while a tokio runtime is active and the `tokio`
//! feature is enabled, a `tracing::warn!` points you at the alternative.

pub use dyn_properties_derive::DynProperties;

mod duration;
#[doc(hidden)]
pub use duration::deserialize_duration_option;
pub use duration::{DurationError, parse_duration};

mod format;
pub use format::Format;
#[cfg(feature = "json")]
pub use format::Json;
#[cfg(feature = "toml")]
pub use format::Toml;

mod error;
mod required;
mod validate;
pub use error::Error;
pub use required::HasRequiredField;
pub use validate::Validate;

mod watcher;
pub use watcher::{ChangeSubscription, PropertyWatcher};

#[cfg(feature = "tokio")]
mod tokio_watcher;

#[cfg(feature = "tokio")]
pub mod tokio {
    //! A tokio-native [`PropertyWatcher`](crate::PropertyWatcher) that spawns no OS
    //! thread — background refresh runs as a `tokio::spawn`'d task, and change
    //! notifications are delivered via `tokio::sync::watch`.
    pub use crate::tokio_watcher::PropertyWatcher;
}

pub mod exports {
    pub use serde;
    pub use tracing;
}
