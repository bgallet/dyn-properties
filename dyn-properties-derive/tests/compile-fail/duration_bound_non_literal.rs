use dyn_properties_derive::DynProperties;

// A stand-in for `dyn_properties::Duration` — the macro classifies a field's type by its
// last path segment name, so a locally-defined type named `Duration` is enough to exercise
// the "#[range] duration bound must be a string literal" rule without depending on the
// facade crate (which would be a circular dev-dependency for this proc-macro crate's own
// tests).
struct Duration;

const MIN: &str = "1s";

#[derive(DynProperties)]
struct Bad {
    #[range(min = MIN, max = "1h")]
    #[default("5s")]
    field: Duration,
}

fn main() {}
