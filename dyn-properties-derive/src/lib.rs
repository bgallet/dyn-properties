use proc_macro::TokenStream;

mod attrs;
mod type_kind;

#[proc_macro_derive(DynProperties, attributes(range, len, duration_range, default))]
pub fn derive_dyn_properties(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
