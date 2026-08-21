use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[required]
    #[default("localhost")]
    field: String,
}

fn main() {}
