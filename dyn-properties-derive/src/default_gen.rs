use proc_macro2::TokenStream;
use quote::quote_spanned;
use syn::spanned::Spanned;

use crate::type_kind::FieldKind;
use crate::ParsedField;

pub fn generate(struct_name: &syn::Ident, fields: &[ParsedField]) -> TokenStream {
    let assignments: Vec<TokenStream> = fields.iter().map(field_default).collect();

    quote_spanned! {struct_name.span()=>
        impl ::std::default::Default for #struct_name {
            fn default() -> Self {
                Self {
                    #(#assignments)*
                }
            }
        }
    }
}

fn field_default(field: &ParsedField) -> TokenStream {
    let ident = &field.ident;
    let field_name = ident.to_string();
    let span = field.field.span();

    let Some(expr) = &field.default else {
        return quote_spanned! {span=> #ident: ::std::default::Default::default(), };
    };

    let effective_kind = match &field.kind {
        FieldKind::Option(inner) => inner.as_ref(),
        other => other,
    };

    let constructed = match effective_kind {
        FieldKind::Numeric => quote_spanned! {span=> #expr },
        FieldKind::String => quote_spanned! {span=>
            { let v: &str = #expr; v.to_string() }
        },
        FieldKind::Duration => quote_spanned! {span=>
            {
                let v: &str = #expr;
                <dyn_properties::Duration as ::std::str::FromStr>::from_str(v)
                    .expect(&::std::format!("invalid #[default] duration literal on field `{}`", #field_name))
            }
        },
        FieldKind::Nested => quote_spanned! {span=> #expr },
        FieldKind::Option(_) => unreachable!("Option<Option<T>> is rejected during type classification"),
    };

    if matches!(field.kind, FieldKind::Option(_)) {
        quote_spanned! {span=> #ident: ::std::option::Option::Some(#constructed), }
    } else {
        quote_spanned! {span=> #ident: #constructed, }
    }
}
