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
//! A required field nested inside another `#[derive(DynProperties)]` struct is only
//! enforced once the file provides that section at all — if the whole section is
//! omitted, the nested struct falls back to its own [`Default`] impl directly, which
//! (being `Default`, not `Deserialize`) never runs its required-field check. If a
//! required field's section might be omitted entirely, mark the nested field itself
//! `#[required]` too, forcing the section's presence:
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
//!     #[required]
//!     secret: SecretConfig,
//! }
//!
//! // Omitting `[secret]` entirely is now also rejected, not just an empty `[secret]`.
//! assert!(Toml::parse::<AppConfig>(b"").is_err());
//! # }
//! ```
//!
//! ## Cargo features
//!
//! Neither format is enabled by default — enable exactly the one(s) you need:
//!
//! - `toml` — adds [`Toml`], parsing config files as TOML.
//! - `json` — adds [`Json`], parsing config files as JSON.
//!
//! Both can be enabled together. With neither enabled, [`Format`] itself is still
//! available — implement it for your own format (YAML, RON, ...) and use
//! `PropertyWatcher<T, YourFormat>` without depending on `toml` or `serde_json` at all.

pub use dyn_properties_derive::DynProperties;

mod duration;
#[doc(hidden)]
pub use duration::deserialize_duration_option;
pub use duration::{ParseDurationError, parse_duration};

mod format;
pub use format::Format;
#[cfg(feature = "json")]
pub use format::Json;
#[cfg(feature = "toml")]
pub use format::Toml;

mod error;
mod validate;
pub use error::Error;
pub use validate::Validate;

mod watcher;
pub use watcher::{ChangeSubscription, PropertyWatcher};

pub mod exports {
    pub use serde;
    pub use tracing;
}
