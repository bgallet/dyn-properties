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
    if matches!(field.kind, FieldKind::Option(_)) {
        quote_spanned! {span=>
            #[serde(default)]
            #ident: #ty,
        }
    } else {
        quote_spanned! {span=>
            #[serde(default)]
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
