use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[range(min = 1, max = 10)]
    field: String,
}

fn main() {}
