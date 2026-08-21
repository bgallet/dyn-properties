use proc_macro2::TokenStream;
use quote::{format_ident, quote_spanned};
use syn::spanned::Spanned;

use crate::ParsedField;
use crate::type_kind::FieldKind;

pub fn generate(struct_name: &syn::Ident, fields: &[ParsedField]) -> TokenStream {
    let helper_name = format_ident!("__{}DynPropertiesHelper", struct_name);
    let struct_name_str = struct_name.to_string();

    let helper_fields: Vec<TokenStream> = fields.iter().map(|f| helper_field(f)).collect();
    let overlay_assignments: Vec<TokenStream> = fields
        .iter()
        .map(|f| overlay_assignment(f, &struct_name_str))
        .collect();
    // Every field's overlay is `.unwrap_or(default_instance.field)` — except `#[required]`
    // fields, which never reference `default_instance` at all. If every field on this
    // struct happens to be `#[required]`, skip constructing it entirely: an unread local
    // would otherwise leak an `unused_variables` warning into the caller's own build.
    let needs_default_instance = fields.iter().any(|f| !f.required);
    let default_instance_binding = if needs_default_instance {
        quote_spanned! {struct_name.span()=>
            let default_instance = <#struct_name as ::std::default::Default>::default();
        }
    } else {
        TokenStream::new()
    };

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
                    // Catches any key that doesn't match one of the named fields above,
                    // instead of serde's default of silently discarding it — logged
                    // below, never a hard error (a config file staying loadable despite
                    // a stray/typo'd key is the whole point).
                    #[serde(flatten)]
                    __dyn_properties_unknown_fields: ::std::collections::HashMap<
                        ::std::string::String,
                        dyn_properties::exports::serde::de::IgnoredAny,
                    >,
                }

                let helper = <#helper_name as dyn_properties::exports::serde::Deserialize>::deserialize(deserializer)?;

                if !helper.__dyn_properties_unknown_fields.is_empty() {
                    let mut unknown_fields: ::std::vec::Vec<&str> = helper
                        .__dyn_properties_unknown_fields
                        .keys()
                        .map(|k| k.as_str())
                        .collect();
                    unknown_fields.sort_unstable();
                    dyn_properties::exports::tracing::warn!(
                        struct_name = #struct_name_str,
                        unknown_fields = ?unknown_fields,
                        "dyn-properties: ignoring unknown field(s)"
                    );
                }

                #default_instance_binding

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

fn overlay_assignment(field: &ParsedField, struct_name_str: &str) -> TokenStream {
    let ident = &field.ident;
    let field_name = ident.to_string();
    let span = field.field.span();
    let missing_required_error = missing_required_error(struct_name_str, &field_name, span);

    if field.required {
        // Compatibility checks in lib.rs already rule out #[required] on an Option<T>
        // field, so this is always the "bare type" overlay arm.
        return quote_spanned! {span=>
            #ident: helper.#ident.ok_or_else(|| #missing_required_error)?,
        };
    }
    if matches!(field.kind, FieldKind::Nested) {
        // Not itself #[required], but its own HasRequiredField const (see
        // required_gen) may say otherwise: a nested struct with a #[required] field of
        // its own must have its section present here too, even though this field
        // wasn't marked #[required] directly. Both branches produce the same type, and
        // the `if` collapses to one arm at compile time (HAS_REQUIRED_FIELD is a
        // `const`), so this costs nothing at runtime either way.
        let ty = &field.field.ty;
        return quote_spanned! {span=>
            #ident: if <#ty as dyn_properties::HasRequiredField>::HAS_REQUIRED_FIELD {
                helper.#ident.ok_or_else(|| #missing_required_error)?
            } else {
                helper.#ident.unwrap_or(default_instance.#ident)
            },
        };
    }
    if matches!(field.kind, FieldKind::Option(_)) {
        quote_spanned! {span=> #ident: helper.#ident.or(default_instance.#ident), }
    } else {
        quote_spanned! {span=> #ident: helper.#ident.unwrap_or(default_instance.#ident), }
    }
}

fn missing_required_error(
    struct_name_str: &str,
    field_name: &str,
    span: proc_macro2::Span,
) -> TokenStream {
    quote_spanned! {span=>
        <D::Error as dyn_properties::exports::serde::de::Error>::custom(::std::format!(
            "{}: field `{}` is required and has no default, but was not found in the config file",
            #struct_name_str, #field_name
        ))
    }
}
