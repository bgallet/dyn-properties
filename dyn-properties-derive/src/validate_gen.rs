use proc_macro2::TokenStream;
use quote::quote_spanned;
use syn::spanned::Spanned;

use crate::attrs::Bound;
use crate::type_kind::FieldKind;
use crate::ParsedField;

pub fn generate(struct_name: &syn::Ident, fields: &[ParsedField]) -> TokenStream {
    let checks: Vec<TokenStream> = fields.iter().map(field_check).collect();

    quote_spanned! {struct_name.span()=>
        impl dyn_properties::Validate for #struct_name {
            #[allow(unused_comparisons, clippy::absurd_extreme_comparisons)]
            fn validate(&self) -> ::std::result::Result<(), dyn_properties::Error> {
                #(#checks)*
                ::std::result::Result::Ok(())
            }
        }
    }
}

fn field_check(field: &ParsedField) -> TokenStream {
    let ident = &field.ident;
    let field_name = ident.to_string();
    let span = field.field.span();

    match (&field.kind, &field.bound) {
        (FieldKind::Numeric, Some(Bound::Range { min, max })) => quote_spanned! {span=>
            if self.#ident < (#min) || self.#ident > (#max) {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("{} is out of range [{}, {}]", self.#ident, #min, #max),
                });
            }
        },
        (FieldKind::String, Some(Bound::Len { min, max })) => quote_spanned! {span=>
            if self.#ident.len() < (#min) || self.#ident.len() > (#max) {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!(
                        "length {} is out of range [{}, {}]", self.#ident.len(), #min, #max
                    ),
                });
            }
        },
        (FieldKind::Duration, Some(Bound::Range { min, max })) => {
            duration_range_check(ident, &field_name, min, max, span)
        }
        (FieldKind::Option(inner), Some(bound)) => option_bound_check(ident, &field_name, inner, bound, span),
        (FieldKind::Nested, _) => quote_spanned! {span=>
            dyn_properties::Validate::validate(&self.#ident).map_err(|e| e.prefixed(#field_name))?;
        },
        (FieldKind::Option(inner), None) if matches!(**inner, FieldKind::Nested) => quote_spanned! {span=>
            if let ::std::option::Option::Some(v) = &self.#ident {
                dyn_properties::Validate::validate(v).map_err(|e| e.prefixed(#field_name))?;
            }
        },
        _ => TokenStream::new(),
    }
}

fn option_bound_check(
    ident: &syn::Ident,
    field_name: &str,
    inner: &FieldKind,
    bound: &Bound,
    span: proc_macro2::Span,
) -> TokenStream {
    if let (FieldKind::Duration, Bound::Range { min, max }) = (inner, bound) {
        return option_duration_range_check(ident, field_name, min, max, span);
    }

    let inner_check = match (inner, bound) {
        (FieldKind::Numeric, Bound::Range { min, max }) => quote_spanned! {span=>
            if *v < (#min) || *v > (#max) {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("{} is out of range [{}, {}]", v, #min, #max),
                });
            }
        },
        (FieldKind::String, Bound::Len { min, max }) => quote_spanned! {span=>
            if v.len() < (#min) || v.len() > (#max) {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("length {} is out of range [{}, {}]", v.len(), #min, #max),
                });
            }
        },
        _ => TokenStream::new(),
    };

    quote_spanned! {span=>
        if let ::std::option::Option::Some(v) = &self.#ident {
            #inner_check
        }
    }
}

fn duration_range_check(
    ident: &syn::Ident,
    field_name: &str,
    min: &syn::Expr,
    max: &syn::Expr,
    span: proc_macro2::Span,
) -> TokenStream {
    quote_spanned! {span=>
        {
            static MIN: ::std::sync::LazyLock<::std::time::Duration> = ::std::sync::LazyLock::new(|| {
                dyn_properties::parse_duration(#min)
                    .expect(&::std::format!("invalid #[range] min literal on field `{}`", #field_name))
            });
            static MAX: ::std::sync::LazyLock<::std::time::Duration> = ::std::sync::LazyLock::new(|| {
                dyn_properties::parse_duration(#max)
                    .expect(&::std::format!("invalid #[range] max literal on field `{}`", #field_name))
            });
            if self.#ident < *MIN || self.#ident > *MAX {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("{:?} is out of range [{:?}, {:?}]", self.#ident, *MIN, *MAX),
                });
            }
        }
    }
}

fn option_duration_range_check(
    ident: &syn::Ident,
    field_name: &str,
    min: &syn::Expr,
    max: &syn::Expr,
    span: proc_macro2::Span,
) -> TokenStream {
    quote_spanned! {span=>
        if let ::std::option::Option::Some(v) = &self.#ident {
            static MIN: ::std::sync::LazyLock<::std::time::Duration> = ::std::sync::LazyLock::new(|| {
                dyn_properties::parse_duration(#min)
                    .expect(&::std::format!("invalid #[range] min literal on field `{}`", #field_name))
            });
            static MAX: ::std::sync::LazyLock<::std::time::Duration> = ::std::sync::LazyLock::new(|| {
                dyn_properties::parse_duration(#max)
                    .expect(&::std::format!("invalid #[range] max literal on field `{}`", #field_name))
            });
            if *v < *MIN || *v > *MAX {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("{:?} is out of range [{:?}, {:?}]", v, *MIN, *MAX),
                });
            }
        }
    }
}
