use syn::{Error, Result, Type};

pub enum FieldKind {
    String,
    Numeric,
    Duration,
    Option(Box<FieldKind>),
    Nested,
}

impl FieldKind {
    /// Whether a field of this kind must carry an explicit `#[default(...)]` attribute
    /// rather than falling back to the type's own `Default::default()`. Only bare
    /// `Duration` fields require this — `Option<Duration>` is exempt, since its fallback
    /// is `None` (via `Option<T>`'s own `Default`), never `Duration::default()`.
    pub fn requires_explicit_default(&self) -> bool {
        matches!(self, FieldKind::Duration)
    }
}

const NUMERIC_TYPES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
];

pub fn classify(ty: &Type) -> Result<FieldKind> {
    let Type::Path(type_path) = ty else {
        return Err(Error::new_spanned(ty, "unsupported field type: expected a path type"));
    };
    let segment = type_path
        .path
        .segments
        .last()
        .ok_or_else(|| Error::new_spanned(ty, "unsupported field type"))?;
    let ident_str = segment.ident.to_string();

    if ident_str == "Option" {
        let inner_ty = extract_generic_arg(segment)
            .ok_or_else(|| Error::new_spanned(ty, "Option must have exactly one type argument"))?;
        let inner_kind = classify(inner_ty)?;
        if matches!(inner_kind, FieldKind::Option(_)) {
            return Err(Error::new_spanned(ty, "nested Option<Option<T>> is not supported"));
        }
        return Ok(FieldKind::Option(Box::new(inner_kind)));
    }

    if ident_str == "String" {
        return Ok(FieldKind::String);
    }

    if ident_str == "Duration" {
        return Ok(FieldKind::Duration);
    }

    if NUMERIC_TYPES.contains(&ident_str.as_str()) {
        return Ok(FieldKind::Numeric);
    }

    Ok(FieldKind::Nested)
}

fn extract_generic_arg(segment: &syn::PathSegment) -> Option<&Type> {
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn classifies_numeric() {
        let ty: Type = parse_quote!(u32);
        assert!(matches!(classify(&ty).unwrap(), FieldKind::Numeric));
    }

    #[test]
    fn classifies_float() {
        let ty: Type = parse_quote!(f64);
        assert!(matches!(classify(&ty).unwrap(), FieldKind::Numeric));
    }

    #[test]
    fn classifies_string() {
        let ty: Type = parse_quote!(String);
        assert!(matches!(classify(&ty).unwrap(), FieldKind::String));
    }

    #[test]
    fn classifies_duration() {
        let ty: Type = parse_quote!(Duration);
        assert!(matches!(classify(&ty).unwrap(), FieldKind::Duration));
    }

    #[test]
    fn classifies_option_of_numeric() {
        let ty: Type = parse_quote!(Option<u32>);
        match classify(&ty).unwrap() {
            FieldKind::Option(inner) => assert!(matches!(*inner, FieldKind::Numeric)),
            _ => panic!("expected Option"),
        }
    }

    #[test]
    fn classifies_nested_struct() {
        let ty: Type = parse_quote!(PoolConfig);
        assert!(matches!(classify(&ty).unwrap(), FieldKind::Nested));
    }

    #[test]
    fn rejects_nested_option() {
        let ty: Type = parse_quote!(Option<Option<u32>>);
        assert!(classify(&ty).is_err());
    }
}
