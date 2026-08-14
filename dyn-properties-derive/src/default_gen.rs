use proc_macro2::TokenStream;

use crate::ParsedField;

pub fn generate(_struct_name: &syn::Ident, _fields: &[ParsedField]) -> TokenStream {
    TokenStream::new()
}
