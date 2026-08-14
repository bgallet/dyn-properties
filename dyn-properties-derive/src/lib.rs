use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Error, Fields, Result};

mod attrs;
mod default_gen;
mod deserialize_gen;
mod type_kind;
mod validate_gen;

use attrs::Bound;
use type_kind::FieldKind;

pub(crate) struct ParsedField<'a> {
    pub field: &'a syn::Field,
    pub ident: syn::Ident,
    pub kind: FieldKind,
    pub bound: Option<Bound>,
    pub default: Option<syn::Expr>,
}

#[proc_macro_derive(DynProperties, attributes(range, len, duration_range, default))]
pub fn derive_dyn_properties(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> Result<TokenStream2> {
    let struct_name = &input.ident;
    let named_fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(Error::new_spanned(
                    input,
                    "DynProperties only supports structs with named fields",
                ))
            }
        },
        _ => return Err(Error::new_spanned(input, "DynProperties only supports structs")),
    };

    let mut parsed_fields = Vec::new();
    for field in named_fields {
        let field_ident = field.ident.clone().expect("named field always has an ident");
        let kind = type_kind::classify(&field.ty)?;
        let field_attrs = attrs::parse_field_attrs(&field.attrs)?;
        if let Some(bound) = &field_attrs.bound {
            check_bound_compatibility(bound, &kind, &field_ident)?;
        }
        check_duration_has_default(&kind, &field_attrs.default, &field_ident)?;
        parsed_fields.push(ParsedField {
            field,
            ident: field_ident,
            kind,
            bound: field_attrs.bound,
            default: field_attrs.default,
        });
    }

    let validate_impl = validate_gen::generate(struct_name, &parsed_fields);
    let default_impl = default_gen::generate(struct_name, &parsed_fields);
    let deserialize_impl = deserialize_gen::generate(struct_name, &parsed_fields);

    Ok(quote! {
        #validate_impl
        #default_impl
        #deserialize_impl
    })
}

fn check_bound_compatibility(bound: &Bound, kind: &FieldKind, field_ident: &syn::Ident) -> Result<()> {
    let effective_kind = match kind {
        FieldKind::Option(inner) => inner.as_ref(),
        other => other,
    };
    let ok = matches!(
        (bound, effective_kind),
        (Bound::Range { .. }, FieldKind::Numeric)
            | (Bound::Len { .. }, FieldKind::String)
            | (Bound::DurationRange { .. }, FieldKind::Duration)
    );
    if ok {
        Ok(())
    } else {
        let attr_name = match bound {
            Bound::Range { .. } => "range",
            Bound::Len { .. } => "len",
            Bound::DurationRange { .. } => "duration_range",
        };
        Err(Error::new_spanned(
            field_ident,
            format!("#[{attr_name}] cannot be used on field `{field_ident}`: incompatible field type"),
        ))
    }
}

fn check_duration_has_default(
    kind: &FieldKind,
    default: &Option<syn::Expr>,
    field_ident: &syn::Ident,
) -> Result<()> {
    if matches!(kind, FieldKind::Duration) && default.is_none() {
        Err(Error::new_spanned(
            field_ident,
            format!(
                "field `{field_ident}` is a Duration and must have a #[default(\"...\")] attribute: \
                 Duration has no implicit default, since a silent zero-duration is rarely the right \
                 fallback for a timeout or interval. Wrap the field in Option<Duration> instead if \
                 \"unset\" (None) is what you actually want."
            ),
        ))
    } else {
        Ok(())
    }
}
