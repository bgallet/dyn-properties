use crate::Error;

pub trait Validate {
    fn validate(&self) -> Result<(), Error>;
}
