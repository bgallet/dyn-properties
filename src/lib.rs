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
