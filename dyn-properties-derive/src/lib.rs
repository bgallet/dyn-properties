use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Result, parse_macro_input};

mod attrs;
mod default_gen;
mod deserialize_gen;
mod duration_syntax;
mod required_gen;
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
    pub required: bool,
}

#[proc_macro_derive(DynProperties, attributes(range, len, default, required))]
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
                ));
            }
        },
        _ => {
            return Err(Error::new_spanned(
                input,
                "DynProperties only supports structs",
            ));
        }
    };

    let mut parsed_fields = Vec::new();
    for field in named_fields {
        let field_ident = field
            .ident
            .clone()
            .expect("named field always has an ident");
        let kind = type_kind::classify(&field.ty)?;
        let field_attrs = attrs::parse_field_attrs(&field.attrs)?;
        if let Some(bound) = &field_attrs.bound {
            check_bound_compatibility(bound, &kind, &field_ident)?;
            check_duration_bound_literal_syntax(bound, &kind)?;
        }
        check_required_compatibility(
            field_attrs.required,
            &field_attrs.default,
            &kind,
            &field_ident,
        )?;
        check_duration_has_default_or_required(
            &kind,
            &field_attrs.default,
            field_attrs.required,
            &field_ident,
        )?;
        check_duration_default_literal_syntax(&field_attrs.default, &kind)?;
        parsed_fields.push(ParsedField {
            field,
            ident: field_ident,
            kind,
            bound: field_attrs.bound,
            default: field_attrs.default,
            required: field_attrs.required,
        });
    }

    let validate_impl = validate_gen::generate(struct_name, &parsed_fields);
    let default_impl = default_gen::generate(struct_name, &parsed_fields);
    let deserialize_impl = deserialize_gen::generate(struct_name, &parsed_fields);
    let has_required_impl = required_gen::generate(struct_name, &parsed_fields);

    Ok(quote! {
        #validate_impl
        #default_impl
        #deserialize_impl
        #has_required_impl
    })
}

/// A field's kind, with an `Option<T>` wrapper stripped down to `T` — the meaningful
/// distinction for compatibility/literal checks that apply equally whether or not a
/// field is optional (an `Option<Duration>` bound behaves the same as a bare `Duration`
/// bound once the value is present).
fn effective_kind(kind: &FieldKind) -> &FieldKind {
    match kind {
        FieldKind::Option(inner) => inner.as_ref(),
        other => other,
    }
}

fn check_bound_compatibility(
    bound: &Bound,
    kind: &FieldKind,
    field_ident: &syn::Ident,
) -> Result<()> {
    let effective_kind = effective_kind(kind);
    let ok = matches!(
        (bound, effective_kind),
        (Bound::Range { .. }, FieldKind::Numeric)
            | (Bound::Range { .. }, FieldKind::Duration)
            | (Bound::Len { .. }, FieldKind::String)
    );
    if ok {
        Ok(())
    } else {
        let attr_name = match bound {
            Bound::Range { .. } => "range",
            Bound::Len { .. } => "len",
        };
        Err(Error::new_spanned(
            field_ident,
            format!(
                "#[{attr_name}] cannot be used on field `{field_ident}`: incompatible field type"
            ),
        ))
    }
}

/// `#[required]` and `#[default(...)]` are mutually exclusive (contradictory: one says
/// "must be present", the other says "here's a fallback if absent"), and `#[required]`
/// cannot be combined with an `Option<T>` field (already means "absence is fine, gives
/// `None`" — `#[required]` on top of that is a contradiction in the other direction).
fn check_required_compatibility(
    required: bool,
    default: &Option<syn::Expr>,
    kind: &FieldKind,
    field_ident: &syn::Ident,
) -> Result<()> {
    if !required {
        return Ok(());
    }
    if default.is_some() {
        return Err(Error::new_spanned(
            field_ident,
            format!(
                "field `{field_ident}` cannot have both #[required] and #[default(...)]: \
                 a required field has no fallback to default to"
            ),
        ));
    }
    if matches!(kind, FieldKind::Option(_)) {
        return Err(Error::new_spanned(
            field_ident,
            format!(
                "#[required] cannot be used on field `{field_ident}`: Option<T> already means \
                 the field may be absent (giving None); use a bare (non-Option) type instead"
            ),
        ));
    }
    Ok(())
}

fn check_duration_has_default_or_required(
    kind: &FieldKind,
    default: &Option<syn::Expr>,
    required: bool,
    field_ident: &syn::Ident,
) -> Result<()> {
    if kind.requires_explicit_default() && default.is_none() && !required {
        Err(Error::new_spanned(
            field_ident,
            format!(
                "field `{field_ident}` is a Duration and must have a #[default(\"...\")] or \
                 #[required] attribute: Duration has no implicit default, since a silent \
                 zero-duration is rarely the right fallback for a timeout or interval. Wrap the \
                 field in Option<Duration> instead if \"unset\" (None) is what you actually want."
            ),
        ))
    } else {
        Ok(())
    }
}

/// If `bound` is a `#[range(min=..,max=..)]` on a Duration-kind field (bare or
/// `Option`-wrapped), requires `min`/`max` to be string literals with valid duration
/// syntax — checked at macro-expansion time so a malformed literal is a compile error,
/// not a panic the first time `validate()` happens to run.
fn check_duration_bound_literal_syntax(bound: &Bound, kind: &FieldKind) -> Result<()> {
    let Bound::Range { min, max } = bound else {
        return Ok(());
    };
    if !matches!(effective_kind(kind), FieldKind::Duration) {
        return Ok(());
    }
    check_duration_literal_expr(min)?;
    check_duration_literal_expr(max)
}

/// If `default` is a `#[default("...")]` on a Duration-kind field (bare or
/// `Option`-wrapped), requires it to be a string literal with valid duration syntax —
/// checked at macro-expansion time for the same reason as bound literals above.
fn check_duration_default_literal_syntax(
    default: &Option<syn::Expr>,
    kind: &FieldKind,
) -> Result<()> {
    let Some(expr) = default else {
        return Ok(());
    };
    if !matches!(effective_kind(kind), FieldKind::Duration) {
        return Ok(());
    }
    check_duration_literal_expr(expr)
}

fn check_duration_literal_expr(expr: &syn::Expr) -> Result<()> {
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(lit_str),
        ..
    }) = expr
    else {
        return Err(Error::new_spanned(
            expr,
            "Duration bounds and defaults must be string literals (e.g. \"30s\"), so they can be validated at compile time",
        ));
    };
    duration_syntax::validate_duration_literal_syntax(&lit_str.value())
        .map_err(|msg| Error::new_spanned(lit_str, msg))
}
