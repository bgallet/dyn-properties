use proc_macro2::TokenStream;
use quote::quote_spanned;
use syn::spanned::Spanned;

use crate::ParsedField;
use crate::type_kind::FieldKind;

/// Generates `impl HasRequiredField for #struct_name`, reporting (as a compile-time
/// constant) whether this struct has a `#[required]` field of its own, or — through a
/// *bare* Nested-kind field only — recursively contains one. This is what lets
/// `deserialize_gen`'s overlay for a bare nested field automatically require that
/// section's presence when the nested struct needs it, without the outer field also
/// needing `#[required]`.
pub fn generate(struct_name: &syn::Ident, fields: &[ParsedField]) -> TokenStream {
    let terms: Vec<TokenStream> = fields.iter().filter_map(field_term).collect();

    let has_required_expr = if terms.is_empty() {
        quote_spanned! {struct_name.span()=> false }
    } else {
        quote_spanned! {struct_name.span()=> #(#terms)||* }
    };

    quote_spanned! {struct_name.span()=>
        #[doc(hidden)]
        impl dyn_properties::HasRequiredField for #struct_name {
            const HAS_REQUIRED_FIELD: bool = #has_required_expr;
        }
    }
}

/// One OR-term contributing to the struct's own transitive "has a required field
/// somewhere" status — `None` if this field contributes nothing.
fn field_term(field: &ParsedField) -> Option<TokenStream> {
    let span = field.field.span();
    if field.required {
        return Some(quote_spanned! {span=> true });
    }
    // Only a *bare* Nested field propagates — `Option<Nested>` is excluded (see the
    // trait's own doc comment for why), and every other kind (String/Numeric/Duration)
    // has no nested fields of its own to propagate from.
    if matches!(field.kind, FieldKind::Nested) {
        let ty = &field.field.ty;
        return Some(quote_spanned! {span=>
            <#ty as dyn_properties::HasRequiredField>::HAS_REQUIRED_FIELD
        });
    }
    None
}
