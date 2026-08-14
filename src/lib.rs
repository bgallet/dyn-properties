pub use dyn_properties_derive::DynProperties;

mod duration;
pub use duration::{Duration, ParseDurationError};

mod error;
mod validate;
pub use error::Error;
pub use validate::Validate;

mod watcher;
pub use watcher::PropertyWatcher;

pub mod exports {
    pub use serde;
}
