use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[required]
    field: Option<String>,
}

fn main() {}
