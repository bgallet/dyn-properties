use proc_macro2::TokenStream;
use quote::{format_ident, quote_spanned};
use syn::spanned::Spanned;

use crate::type_kind::FieldKind;
use crate::ParsedField;

pub fn generate(struct_name: &syn::Ident, fields: &[ParsedField]) -> TokenStream {
    let helper_name = format_ident!("__{}DynPropertiesHelper", struct_name);

    let helper_fields: Vec<TokenStream> = fields.iter().map(|f| helper_field(f)).collect();
    let overlay_assignments: Vec<TokenStream> = fields.iter().map(|f| overlay_assignment(f)).collect();

    quote_spanned! {struct_name.span()=>
        impl<'de> dyn_properties::exports::serde::Deserialize<'de> for #struct_name {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: dyn_properties::exports::serde::Deserializer<'de>,
            {
                #[derive(dyn_properties::exports::serde::Deserialize)]
                #[serde(crate = "dyn_properties::exports::serde")]
                struct #helper_name {
                    #(#helper_fields)*
                }

                let helper = <#helper_name as dyn_properties::exports::serde::Deserialize>::deserialize(deserializer)?;
                let default_instance = <#struct_name as ::std::default::Default>::default();

                ::std::result::Result::Ok(#struct_name {
                    #(#overlay_assignments)*
                })
            }
        }
    }
}

fn helper_field(field: &ParsedField) -> TokenStream {
    let ident = &field.ident;
    let ty = &field.field.ty;
    let span = field.field.span();

    // `std::time::Duration` has no `Deserialize` impl of its own (see `src/duration.rs`),
    // so Duration-kind fields (bare or `Option`-wrapped — both end up as `Option<Duration>`
    // in the helper, see below) need an explicit deserializer wired in. Every other kind
    // (String, numeric, nested derived structs) already implements `Deserialize` and needs
    // nothing extra here.
    let effective_kind = match &field.kind {
        FieldKind::Option(inner) => inner.as_ref(),
        other => other,
    };
    let deserialize_with = if matches!(effective_kind, FieldKind::Duration) {
        quote_spanned! {span=>
            #[serde(deserialize_with = "dyn_properties::deserialize_duration_option")]
        }
    } else {
        TokenStream::new()
    };

    if matches!(field.kind, FieldKind::Option(_)) {
        quote_spanned! {span=>
            #[serde(default)]
            #deserialize_with
            #ident: #ty,
        }
    } else {
        quote_spanned! {span=>
            #[serde(default)]
            #deserialize_with
            #ident: ::std::option::Option<#ty>,
        }
    }
}

fn overlay_assignment(field: &ParsedField) -> TokenStream {
    let ident = &field.ident;
    let span = field.field.span();
    if matches!(field.kind, FieldKind::Option(_)) {
        quote_spanned! {span=> #ident: helper.#ident.or(default_instance.#ident), }
    } else {
        quote_spanned! {span=> #ident: helper.#ident.unwrap_or(default_instance.#ident), }
    }
}
