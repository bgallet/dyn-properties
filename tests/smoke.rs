use dyn_properties::DynProperties;

#[derive(DynProperties)]
struct Empty {
    x: u32,
}

#[test]
fn derive_compiles_on_a_plain_struct() {
    let _ = Empty { x: 1 };
}
