use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[opaque]
    #[default("hi")]
    field: String,
}

fn main() {}
