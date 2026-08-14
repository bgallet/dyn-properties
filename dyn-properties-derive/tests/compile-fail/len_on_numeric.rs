use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[len(min = 1, max = 10)]
    field: u32,
}

fn main() {}
