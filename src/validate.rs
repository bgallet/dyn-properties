use crate::Error;

/// Checks that a value's fields satisfy their declared bounds (`#[range]`, `#[len]`,
/// `#[duration_range]`) after it has been deserialized.
///
/// This trait is implemented for you by `#[derive(DynProperties)]`; it isn't meant to
/// be implemented by hand. [`PropertyWatcher`](crate::PropertyWatcher) calls it after
/// every TOML parse (initial load and each reload tick) and discards the new value,
/// keeping the previous one, if validation fails.
pub trait Validate {
    /// Returns `Ok(())` if every bounded field is within range, or the first
    /// [`Error::Validation`] encountered otherwise.
    fn validate(&self) -> Result<(), Error>;
}
