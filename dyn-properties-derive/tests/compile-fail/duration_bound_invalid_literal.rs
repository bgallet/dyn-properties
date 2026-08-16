use dyn_properties_derive::DynProperties;

// A stand-in for `dyn_properties::Duration` — the macro classifies a field's type by its
// last path segment name, so a locally-defined type named `Duration` is enough to exercise
// the "#[range] duration literal must have valid syntax" rule without depending on the
// facade crate (which would be a circular dev-dependency for this proc-macro crate's own
// tests).
struct Duration;

#[derive(DynProperties)]
struct Bad {
    #[range(min = "not-a-duration", max = "1h")]
    #[default("5s")]
    field: Duration,
}

fn main() {}
