use dyn_properties_derive::DynProperties;

// A stand-in for `dyn_properties::Duration` — the macro classifies a field's type by its
// last path segment name, so a locally-defined type named `Duration` is enough to exercise
// the "#[len] is not valid on a Duration field" rule without depending on the facade crate
// (which would be a circular dev-dependency for this proc-macro crate's own tests).
struct Duration;

#[derive(DynProperties)]
struct Bad {
    #[len(min = 1, max = 10)]
    field: Duration,
}

fn main() {}
