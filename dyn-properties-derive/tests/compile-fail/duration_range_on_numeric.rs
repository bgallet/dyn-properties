use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[duration_range(min = "1s", max = "10s")]
    field: u32,
}

fn main() {}
