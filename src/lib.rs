pub use dyn_properties_derive::DynProperties;

mod duration;
pub use duration::{Duration, ParseDurationError};

pub mod exports {
    pub use serde;
}
