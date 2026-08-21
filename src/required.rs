/// Implementation detail of `#[derive(DynProperties)]`'s `#[required]` support — not
/// meant to be implemented or called directly.
///
/// Every `#[derive(DynProperties)]` struct gets an impl of this trait automatically,
/// reporting whether it — or, recursively, any *bare* (non-`Option`-wrapped) nested
/// `DynProperties` field of its own — contains a `#[required]` field. This is what
/// lets a `#[required]` field nested inside another struct enforce its own section's
/// presence automatically wherever that struct is used as a field, without needing the
/// outer field to *also* be marked `#[required]`. `Option<Nested>` fields are excluded
/// from this propagation: wrapping a nested field in `Option` is an explicit "this
/// whole section is optional" signal that wins over transitive requirement.
#[doc(hidden)]
pub trait HasRequiredField {
    const HAS_REQUIRED_FIELD: bool;
}
