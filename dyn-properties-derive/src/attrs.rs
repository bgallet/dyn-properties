use syn::{Attribute, Error, Expr, Result};

pub enum Bound {
    /// A value-comparison bound: `self.field < min || self.field > max`. Used for both
    /// numeric fields (min/max are numeric literals) and `Duration` fields (min/max are
    /// string literals parsed via `dyn_properties::parse_duration`) — which comparison
    /// applies is decided later, from the field's own `FieldKind`, not from the attribute
    /// name.
    Range { min: Expr, max: Expr },
    /// A length-comparison bound: `self.field.len() < min || self.field.len() > max`.
    /// Used for `String` fields. Kept separate from `Range` since it measures a derived
    /// property (length) rather than the field's own value.
    Len { min: Expr, max: Expr },
}

pub struct FieldAttrs {
    pub bound: Option<Bound>,
    pub default: Option<Expr>,
}

fn parse_min_max(attr: &Attribute) -> Result<(Expr, Expr)> {
    let mut min = None;
    let mut max = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("min") {
            min = Some(meta.value()?.parse::<Expr>()?);
            Ok(())
        } else if meta.path.is_ident("max") {
            max = Some(meta.value()?.parse::<Expr>()?);
            Ok(())
        } else {
            Err(meta.error("expected `min` or `max`"))
        }
    })?;
    let min = min.ok_or_else(|| Error::new_spanned(attr, "missing `min` in attribute"))?;
    let max = max.ok_or_else(|| Error::new_spanned(attr, "missing `max` in attribute"))?;
    Ok((min, max))
}

pub fn parse_field_attrs(attrs: &[Attribute]) -> Result<FieldAttrs> {
    let mut bound: Option<Bound> = None;
    let mut default: Option<Expr> = None;

    for attr in attrs {
        if attr.path().is_ident("range") || attr.path().is_ident("len") {
            if bound.is_some() {
                return Err(Error::new_spanned(
                    attr,
                    "only one of #[range], #[len] is allowed per field",
                ));
            }
            let (min, max) = parse_min_max(attr)?;
            bound = Some(if attr.path().is_ident("range") {
                Bound::Range { min, max }
            } else {
                Bound::Len { min, max }
            });
        } else if attr.path().is_ident("default") {
            default = Some(attr.parse_args()?);
        }
    }

    Ok(FieldAttrs { bound, default })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{Data, DeriveInput, Fields};

    fn first_field_attrs(input: proc_macro2::TokenStream) -> Result<FieldAttrs> {
        let derive_input: DeriveInput = syn::parse2(input).unwrap();
        let fields = match derive_input.data {
            Data::Struct(s) => match s.fields {
                Fields::Named(named) => named.named,
                _ => panic!("expected named fields"),
            },
            _ => panic!("expected struct"),
        };
        let field = fields.into_iter().next().unwrap();
        parse_field_attrs(&field.attrs)
    }

    #[test]
    fn parses_range_bound() {
        let attrs = first_field_attrs(quote::quote! {
            struct Foo { #[range(min = 1, max = 100)] field: u32 }
        })
        .unwrap();
        assert!(matches!(attrs.bound, Some(Bound::Range { .. })));
    }

    #[test]
    fn parses_len_bound() {
        let attrs = first_field_attrs(quote::quote! {
            struct Foo { #[len(min = 3, max = 64)] field: String }
        })
        .unwrap();
        assert!(matches!(attrs.bound, Some(Bound::Len { .. })));
    }

    #[test]
    fn parses_range_bound_with_string_literals_for_duration() {
        let attrs = first_field_attrs(quote::quote! {
            struct Foo { #[range(min = "100ms", max = "30s")] field: Duration }
        })
        .unwrap();
        assert!(matches!(attrs.bound, Some(Bound::Range { .. })));
    }

    #[test]
    fn parses_default_value() {
        let attrs = first_field_attrs(quote::quote! {
            struct Foo { #[default(5432)] field: u16 }
        })
        .unwrap();
        assert!(attrs.default.is_some());
    }

    #[test]
    fn no_attrs_gives_none() {
        let attrs = first_field_attrs(quote::quote! {
            struct Foo { field: u16 }
        })
        .unwrap();
        assert!(attrs.bound.is_none());
        assert!(attrs.default.is_none());
    }

    #[test]
    fn duplicate_bound_attrs_error() {
        let result = first_field_attrs(quote::quote! {
            struct Foo {
                #[range(min = 1, max = 2)]
                #[len(min = 1, max = 2)]
                field: u32
            }
        });
        assert!(result.is_err());
    }

    #[test]
    fn missing_max_errors() {
        let result = first_field_attrs(quote::quote! {
            struct Foo { #[range(min = 1)] field: u32 }
        });
        assert!(result.is_err());
    }
}
