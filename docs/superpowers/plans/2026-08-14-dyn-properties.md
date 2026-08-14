# dyn-properties Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `dyn-properties` Rust library: a derive macro that turns a plain struct into a bounds-validated, defaulted, atomically-hot-reloaded view of a TOML file.

**Architecture:** Two-crate workspace. `dyn-properties-derive` (proc-macro crate) parses `#[range]`/`#[len]`/`#[duration_range]`/`#[default]` field attributes and generates `Validate`, `Default`, and `serde::Deserialize` impls for the annotated struct. `dyn-properties` (facade crate) supplies the `Duration` type, `Error`/`Validate` types the generated code references, re-exports `serde` so callers don't need it directly, re-exports the derive macro, and implements `PropertyWatcher<T>` (ArcSwap + a `tokio` background task) for atomic polling reload.

**Tech Stack:** Rust (edition 2024, requires Rust ≥ 1.80 for `std::sync::LazyLock`), `syn`/`quote`/`proc-macro2` (macro), `serde`/`toml` (parsing), `arc-swap` (atomic publish), `tokio` (background task), `tracing` (reload-failure logging), `trybuild` (compile-fail tests), `tempfile`/`tracing-test` (dev-only test support).

**Spec:** `docs/superpowers/specs/2026-08-14-dyn-properties-design.md`

## Global Constraints

- No `unsafe` code anywhere in either crate (per spec's Send/Sync section — auto-trait propagation is sufficient).
- Callers add exactly one dependency (`dyn-properties`) and one derive (`#[derive(DynProperties)]`) — never `serde`/`toml` directly, never a second derive.
- Field types supported: `String`, `i8/i16/i32/i64/i128/isize/u8/u16/u32/u64/u128/usize/f32/f64`, `dyn_properties::Duration`, `Option<T>` of any of those, and nested `#[derive(DynProperties)]` structs. `Option<Option<T>>` is rejected at compile time.
- A reload (initial or interval-driven) either fully succeeds (parse + validate) or is fully discarded — never a partial/per-field merge.
- Every step below that runs tests must actually be run and pass before moving to the next step — do not mark a step done on code inspection alone.

---

## Task 1: Workspace scaffold

**Files:**
- Modify: `Cargo.toml` (root)
- Delete: `src/main.rs`
- Create: `src/lib.rs`
- Create: `dyn-properties-derive/Cargo.toml`
- Create: `dyn-properties-derive/src/lib.rs`
- Test: `tests/smoke.rs`

**Interfaces:**
- Consumes: nothing (first task).
- Produces: workspace builds; `dyn_properties::DynProperties` derive macro exists (no-op — emits nothing); `dyn_properties::exports::serde` re-export.

- [ ] **Step 1: Rewrite the root `Cargo.toml` as both the `dyn-properties` package manifest and the workspace manifest**

```toml
[package]
name = "dyn-properties"
version = "0.1.0"
edition = "2024"

[dependencies]
dyn-properties-derive = { path = "dyn-properties-derive", version = "0.1.0" }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
arc-swap = "1"
tokio = { version = "1", features = ["rt", "time", "fs"] }
tracing = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros", "time"] }
tempfile = "3"
tracing-test = "0.2"

[workspace]
members = ["dyn-properties-derive"]
```

- [ ] **Step 2: Remove the placeholder binary and add the crate root**

```bash
rm src/main.rs
```

Create `src/lib.rs`:

```rust
pub use dyn_properties_derive::DynProperties;

pub mod exports {
    pub use serde;
}
```

- [ ] **Step 3: Create the derive crate manifest**

Create `dyn-properties-derive/Cargo.toml`:

```toml
[package]
name = "dyn-properties-derive"
version = "0.1.0"
edition = "2024"

[lib]
proc-macro = true

[dependencies]
syn = { version = "2", features = ["full"] }
quote = "1"
proc-macro2 = "1"

[dev-dependencies]
trybuild = "1"
```

- [ ] **Step 4: Create a no-op derive macro**

Create `dyn-properties-derive/src/lib.rs`:

```rust
use proc_macro::TokenStream;

#[proc_macro_derive(DynProperties, attributes(range, len, duration_range, default))]
pub fn derive_dyn_properties(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
```

- [ ] **Step 5: Write a smoke test proving the workspace and derive macro compile together**

Create `tests/smoke.rs`:

```rust
use dyn_properties::DynProperties;

#[derive(DynProperties)]
struct Empty {
    x: u32,
}

#[test]
fn derive_compiles_on_a_plain_struct() {
    let _ = Empty { x: 1 };
}
```

- [ ] **Step 6: Build and test the whole workspace**

Run: `cargo test --workspace`
Expected: compiles cleanly, `derive_compiles_on_a_plain_struct` passes.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs dyn-properties-derive tests/smoke.rs
git rm src/main.rs
git commit -m "$(cat <<'EOF'
Scaffold dyn-properties workspace with a no-op derive macro

Root crate becomes the dyn-properties library facade; adds the
dyn-properties-derive proc-macro crate as a workspace member.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `Duration` type

**Files:**
- Create: `src/duration.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `dyn_properties::Duration` (newtype over `std::time::Duration`; `Deref<Target = std::time::Duration>`, `Display`, `Clone + Copy + PartialEq + Eq + PartialOrd + Ord + Hash + Debug`, `FromStr<Err = ParseDurationError>`, `serde::Deserialize`). `dyn_properties::ParseDurationError` (`Display + std::error::Error`).

- [ ] **Step 1: Write the failing unit tests**

Create `src/duration.rs`:

```rust
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use std::time::Duration as StdDuration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(StdDuration);

impl Deref for Duration {
    type Target = StdDuration;
    fn deref(&self) -> &StdDuration {
        &self.0
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseDurationError(String);

impl fmt::Display for ParseDurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid duration string `{}`: expected digits followed by one of ms, s, m, h, d",
            self.0
        )
    }
}

impl std::error::Error for ParseDurationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_milliseconds() {
        assert_eq!("100ms".parse::<Duration>().unwrap().0, StdDuration::from_millis(100));
    }

    #[test]
    fn parses_seconds() {
        assert_eq!("30s".parse::<Duration>().unwrap().0, StdDuration::from_secs(30));
    }

    #[test]
    fn parses_minutes() {
        assert_eq!("5m".parse::<Duration>().unwrap().0, StdDuration::from_secs(300));
    }

    #[test]
    fn parses_hours() {
        assert_eq!("2h".parse::<Duration>().unwrap().0, StdDuration::from_secs(7200));
    }

    #[test]
    fn parses_days() {
        assert_eq!("1d".parse::<Duration>().unwrap().0, StdDuration::from_secs(86400));
    }

    #[test]
    fn rejects_invalid_suffix() {
        assert!("100xyz".parse::<Duration>().is_err());
    }

    #[test]
    fn rejects_missing_digits() {
        assert!("s".parse::<Duration>().is_err());
    }

    #[test]
    fn rejects_negative_values() {
        assert!("-5s".parse::<Duration>().is_err());
    }

    #[test]
    fn rejects_empty_string() {
        assert!("".parse::<Duration>().is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (no `FromStr` impl yet)**

Run: `cargo test -p dyn-properties --lib duration::tests`
Expected: FAIL to compile — `the trait bound Duration: FromStr is not satisfied`.

- [ ] **Step 3: Implement `FromStr` for `Duration`**

Add to `src/duration.rs` (below the `ParseDurationError` impls):

```rust
impl FromStr for Duration {
    type Err = ParseDurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let unit_start = s
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| ParseDurationError(s.to_string()))?;
        let (digits, unit) = s.split_at(unit_start);
        if digits.is_empty() {
            return Err(ParseDurationError(s.to_string()));
        }
        let value: u64 = digits.parse().map_err(|_| ParseDurationError(s.to_string()))?;
        let std_duration = match unit {
            "ms" => StdDuration::from_millis(value),
            "s" => StdDuration::from_secs(value),
            "m" => StdDuration::from_secs(value * 60),
            "h" => StdDuration::from_secs(value * 3600),
            "d" => StdDuration::from_secs(value * 86400),
            _ => return Err(ParseDurationError(s.to_string())),
        };
        Ok(Duration(std_duration))
    }
}
```

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --lib duration::tests`
Expected: PASS, all 9 tests green.

- [ ] **Step 5: Add the `serde::Deserialize` impl**

Add to `src/duration.rs`:

```rust
impl<'de> serde::Deserialize<'de> for Duration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
```

- [ ] **Step 6: Wire the module into the crate root**

Add to `src/lib.rs` (above `pub mod exports`):

```rust
mod duration;
pub use duration::{Duration, ParseDurationError};
```

- [ ] **Step 7: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/duration.rs src/lib.rs
git commit -m "$(cat <<'EOF'
Add Duration type with string parsing and Deserialize

Parses "<digits><unit>" (ms, s, m, h, d) into a std::time::Duration
newtype used by bounded and defaulted config fields.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `Error` type, `Validate` trait, and `exports` module

**Files:**
- Create: `src/error.rs`
- Create: `src/validate.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `dyn_properties::Error` (`enum { Io(std::io::Error), TomlParse(toml::de::Error), Validation { field_path: String, reason: String } }`, `Display`, `std::error::Error`, `From<std::io::Error>`, `From<toml::de::Error>`, method `fn prefixed(self, parent_field: &str) -> Self`). `dyn_properties::Validate` (`trait { fn validate(&self) -> Result<(), Error>; }`).

- [ ] **Step 1: Write the failing unit tests for `Error::prefixed`**

Create `src/error.rs`:

```rust
use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    TomlParse(toml::de::Error),
    Validation { field_path: String, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_prepends_parent_field_to_validation_error() {
        let err = Error::Validation {
            field_path: "idle".to_string(),
            reason: "too big".to_string(),
        };
        let prefixed = err.prefixed("pool");
        match prefixed {
            Error::Validation { field_path, .. } => assert_eq!(field_path, "pool.idle"),
            _ => panic!("expected Validation variant"),
        }
    }

    #[test]
    fn prefixed_leaves_non_validation_variants_unchanged() {
        let parse_err = toml::from_str::<toml::Value>("not valid = [").unwrap_err();
        let err = Error::TomlParse(parse_err);
        let prefixed = err.prefixed("pool");
        assert!(matches!(prefixed, Error::TomlParse(_)));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (no `prefixed` method yet)**

Run: `cargo test -p dyn-properties --lib error::tests`
Expected: FAIL to compile — `no method named prefixed found`.

- [ ] **Step 3: Implement `prefixed`, `Display`, `std::error::Error`, and the `From` impls**

Add to `src/error.rs`:

```rust
impl Error {
    pub fn prefixed(self, parent_field: &str) -> Self {
        match self {
            Error::Validation { field_path, reason } => Error::Validation {
                field_path: format!("{parent_field}.{field_path}"),
                reason,
            },
            other => other,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::TomlParse(e) => write!(f, "TOML parse error: {e}"),
            Error::Validation { field_path, reason } => write!(f, "{field_path}: {reason}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::TomlParse(e) => Some(e),
            Error::Validation { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::TomlParse(e)
    }
}
```

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --lib error::tests`
Expected: PASS, both tests green.

- [ ] **Step 5: Add the `Validate` trait**

Create `src/validate.rs`:

```rust
use crate::Error;

pub trait Validate {
    fn validate(&self) -> Result<(), Error>;
}
```

- [ ] **Step 6: Wire both modules into the crate root**

Add to `src/lib.rs`:

```rust
mod error;
mod validate;
pub use error::Error;
pub use validate::Validate;
```

- [ ] **Step 7: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/error.rs src/validate.rs src/lib.rs
git commit -m "$(cat <<'EOF'
Add Error type and Validate trait

Error carries a dot-joined field_path for nested validation failures
via prefixed(); Validate is the trait generated code implements.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Field-attribute parsing (`attrs.rs`)

**Files:**
- Create: `dyn-properties-derive/src/attrs.rs`
- Modify: `dyn-properties-derive/src/lib.rs`

**Interfaces:**
- Consumes: nothing new (pure `syn` parsing).
- Produces: `attrs::Bound` (`enum { Range { min: syn::Expr, max: syn::Expr }, Len { min: syn::Expr, max: syn::Expr }, DurationRange { min: syn::Expr, max: syn::Expr } }`), `attrs::FieldAttrs { bound: Option<Bound>, default: Option<syn::Expr> }`, `attrs::parse_field_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs>`.

- [ ] **Step 1: Write the failing unit tests**

Create `dyn-properties-derive/src/attrs.rs`:

```rust
use syn::{Attribute, Error, Expr, Result};

pub enum Bound {
    Range { min: Expr, max: Expr },
    Len { min: Expr, max: Expr },
    DurationRange { min: Expr, max: Expr },
}

pub struct FieldAttrs {
    pub bound: Option<Bound>,
    pub default: Option<Expr>,
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
    fn parses_duration_range_bound() {
        let attrs = first_field_attrs(quote::quote! {
            struct Foo { #[duration_range(min = "100ms", max = "30s")] field: Duration }
        })
        .unwrap();
        assert!(matches!(attrs.bound, Some(Bound::DurationRange { .. })));
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
```

- [ ] **Step 2: Add `quote` as a dev-dependency of the derive crate**

Edit `dyn-properties-derive/Cargo.toml`, adding to `[dev-dependencies]`:

```toml
quote = "1"
```

- [ ] **Step 3: Run the tests to verify they fail (no `parse_field_attrs` yet)**

Run: `cargo test -p dyn-properties-derive --lib attrs::tests`
Expected: FAIL to compile — `cannot find function parse_field_attrs`.

- [ ] **Step 4: Implement `parse_field_attrs`**

Add to `dyn-properties-derive/src/attrs.rs` (above the test module):

```rust
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
        if attr.path().is_ident("range") || attr.path().is_ident("len") || attr.path().is_ident("duration_range") {
            if bound.is_some() {
                return Err(Error::new_spanned(
                    attr,
                    "only one of #[range], #[len], #[duration_range] is allowed per field",
                ));
            }
            let (min, max) = parse_min_max(attr)?;
            bound = Some(if attr.path().is_ident("range") {
                Bound::Range { min, max }
            } else if attr.path().is_ident("len") {
                Bound::Len { min, max }
            } else {
                Bound::DurationRange { min, max }
            });
        } else if attr.path().is_ident("default") {
            default = Some(attr.parse_args()?);
        }
    }

    Ok(FieldAttrs { bound, default })
}
```

- [ ] **Step 5: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties-derive --lib attrs::tests`
Expected: PASS, all 6 tests green.

- [ ] **Step 6: Wire the module into the derive crate root**

Add to `dyn-properties-derive/src/lib.rs` (above the existing `derive_dyn_properties` function):

```rust
mod attrs;
```

- [ ] **Step 7: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS (the new `attrs` module is unused by the still-no-op derive function, which is fine — it will only produce a warning, not an error).

- [ ] **Step 8: Commit**

```bash
git add dyn-properties-derive/src/attrs.rs dyn-properties-derive/src/lib.rs dyn-properties-derive/Cargo.toml
git commit -m "$(cat <<'EOF'
Add field-attribute parsing for range/len/duration_range/default

Pure syn-based parsing, not yet wired into the derive macro's output.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Field-type classification (`type_kind.rs`)

**Files:**
- Create: `dyn-properties-derive/src/type_kind.rs`
- Modify: `dyn-properties-derive/src/lib.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `type_kind::FieldKind` (`enum { String, Numeric, Duration, Option(Box<FieldKind>), Nested }`), `type_kind::classify(ty: &syn::Type) -> syn::Result<FieldKind>`.

- [ ] **Step 1: Write the failing unit tests**

Create `dyn-properties-derive/src/type_kind.rs`:

```rust
use syn::{Error, Result, Type};

pub enum FieldKind {
    String,
    Numeric,
    Duration,
    Option(Box<FieldKind>),
    Nested,
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
```

- [ ] **Step 2: Run the tests to verify they fail (no `classify` function yet)**

Run: `cargo test -p dyn-properties-derive --lib type_kind::tests`
Expected: FAIL to compile — `cannot find function classify`.

- [ ] **Step 3: Implement `classify`**

Add to `dyn-properties-derive/src/type_kind.rs` (above the test module):

```rust
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
```

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties-derive --lib type_kind::tests`
Expected: PASS, all 7 tests green.

- [ ] **Step 5: Wire the module into the derive crate root**

Add to `dyn-properties-derive/src/lib.rs`:

```rust
mod type_kind;
```

- [ ] **Step 6: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add dyn-properties-derive/src/type_kind.rs dyn-properties-derive/src/lib.rs
git commit -m "$(cat <<'EOF'
Add field-type classification for the derive macro

Classifies each field's syn::Type as String, Numeric, Duration,
Option<T>, or a nested derived struct, by last path segment name.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Derive macro skeleton, compatibility checks, and trybuild tests

**Files:**
- Modify: `dyn-properties-derive/src/lib.rs`
- Create: `dyn-properties-derive/src/validate_gen.rs` (stub)
- Create: `dyn-properties-derive/src/default_gen.rs` (stub)
- Create: `dyn-properties-derive/src/deserialize_gen.rs` (stub)
- Create: `dyn-properties-derive/tests/trybuild.rs`
- Create: `dyn-properties-derive/tests/compile-fail/range_on_string.rs`
- Create: `dyn-properties-derive/tests/compile-fail/len_on_numeric.rs`
- Create: `dyn-properties-derive/tests/compile-fail/duration_range_on_numeric.rs`

**Interfaces:**
- Consumes: `attrs::{Bound, FieldAttrs, parse_field_attrs}` (Task 4), `type_kind::{FieldKind, classify}` (Task 5).
- Produces: `pub(crate) struct ParsedField<'a> { field: &'a syn::Field, ident: syn::Ident, kind: FieldKind, bound: Option<Bound>, default: Option<syn::Expr> }`; the real `derive_dyn_properties`/`expand` pipeline (parses fields, checks bound/type compatibility, emits `compile_error!` on mismatch); `validate_gen::generate`, `default_gen::generate`, `deserialize_gen::generate` all with signature `fn generate(struct_name: &syn::Ident, fields: &[ParsedField]) -> proc_macro2::TokenStream` (stubs return `TokenStream::new()` for now, replaced in Tasks 7-9).

- [ ] **Step 1: Create stub codegen modules**

Create `dyn-properties-derive/src/validate_gen.rs`:

```rust
use proc_macro2::TokenStream;

use crate::ParsedField;

pub fn generate(_struct_name: &syn::Ident, _fields: &[ParsedField]) -> TokenStream {
    TokenStream::new()
}
```

Create `dyn-properties-derive/src/default_gen.rs` (same body, different file):

```rust
use proc_macro2::TokenStream;

use crate::ParsedField;

pub fn generate(_struct_name: &syn::Ident, _fields: &[ParsedField]) -> TokenStream {
    TokenStream::new()
}
```

Create `dyn-properties-derive/src/deserialize_gen.rs` (same body, different file):

```rust
use proc_macro2::TokenStream;

use crate::ParsedField;

pub fn generate(_struct_name: &syn::Ident, _fields: &[ParsedField]) -> TokenStream {
    TokenStream::new()
}
```

- [ ] **Step 2: Replace the no-op derive function with the real pipeline**

Replace the entire contents of `dyn-properties-derive/src/lib.rs`:

```rust
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
```

- [ ] **Step 3: Run the workspace test suite to confirm nothing broke**

Run: `cargo test --workspace`
Expected: PASS (derive still emits no trait impls, so `tests/smoke.rs` still passes since it never calls `Validate`/`Default`/`Deserialize`).

- [ ] **Step 4: Add the trybuild harness and compile-fail fixtures**

Add to `dyn-properties-derive/Cargo.toml` `[dev-dependencies]`:

```toml
trybuild = "1"
```

Create `dyn-properties-derive/tests/trybuild.rs`:

```rust
#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}
```

Create `dyn-properties-derive/tests/compile-fail/range_on_string.rs`:

```rust
use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[range(min = 1, max = 10)]
    field: String,
}

fn main() {}
```

Create `dyn-properties-derive/tests/compile-fail/len_on_numeric.rs`:

```rust
use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[len(min = 1, max = 10)]
    field: u32,
}

fn main() {}
```

Create `dyn-properties-derive/tests/compile-fail/duration_range_on_numeric.rs`:

```rust
use dyn_properties_derive::DynProperties;

#[derive(DynProperties)]
struct Bad {
    #[duration_range(min = "1s", max = "10s")]
    field: u32,
}

fn main() {}
```

- [ ] **Step 5: Generate the expected `.stderr` snapshots**

Run: `TRYBUILD=overwrite cargo test -p dyn-properties-derive --test trybuild`
Expected: trybuild compiles each fixture, captures the real `rustc` output, and writes `tests/compile-fail/*.stderr` next to each `.rs` file.

- [ ] **Step 6: Inspect the generated `.stderr` files for the right error**

Run: `grep -l "incompatible field type" dyn-properties-derive/tests/compile-fail/*.stderr`
Expected: all three `.stderr` files listed. If any is missing the phrase, re-check Step 2's `check_bound_compatibility` message and re-run Step 5.

- [ ] **Step 7: Run the trybuild test normally (without overwrite) to confirm it's stable**

Run: `cargo test -p dyn-properties-derive --test trybuild`
Expected: PASS.

- [ ] **Step 8: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add dyn-properties-derive/src dyn-properties-derive/tests dyn-properties-derive/Cargo.toml
git commit -m "$(cat <<'EOF'
Wire the derive macro pipeline and add compile-fail tests

expand() now parses every field's type and attributes and rejects
attribute/type mismatches (e.g. #[range] on a String) with a clear
compile_error!. Codegen modules are still stubs (Tasks 7-9).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `Validate` codegen

**Files:**
- Modify: `dyn-properties-derive/src/validate_gen.rs`
- Create: `tests/validate.rs`

**Interfaces:**
- Consumes: `ParsedField` (Task 6), `dyn_properties::{Validate, Error, Duration}` (Tasks 2-3).
- Produces: for every `#[derive(DynProperties)]` struct, a real `impl dyn_properties::Validate for StructName` that checks `#[range]`/`#[len]`/`#[duration_range]` bounds (including inside `Option<T>`) and recurses into nested-struct fields via `Error::prefixed`.

- [ ] **Step 1: Write the failing integration tests**

Create `tests/validate.rs`:

```rust
use dyn_properties::{DynProperties, Duration, Validate};

#[derive(DynProperties)]
struct PoolConfig {
    #[range(min = 0, max = 50)]
    idle: u32,
}

#[derive(DynProperties)]
struct DbConfig {
    #[len(min = 3, max = 64)]
    host: String,

    #[range(min = 1, max = 65535)]
    port: u16,

    #[duration_range(min = "100ms", max = "30s")]
    connect_timeout: Duration,

    #[range(min = 1, max = 100)]
    max_conns: Option<u32>,

    pool: PoolConfig,
}

fn valid_config() -> DbConfig {
    DbConfig {
        host: "localhost".to_string(),
        port: 5432,
        connect_timeout: "5s".parse().unwrap(),
        max_conns: Some(10),
        pool: PoolConfig { idle: 5 },
    }
}

#[test]
fn valid_values_pass() {
    assert!(valid_config().validate().is_ok());
}

#[test]
fn boundary_values_pass() {
    let mut cfg = valid_config();
    cfg.port = 65535;
    cfg.host = "a".repeat(64);
    cfg.connect_timeout = "30s".parse().unwrap();
    assert!(cfg.validate().is_ok());
}

#[test]
fn numeric_out_of_range_fails_with_field_path() {
    let mut cfg = valid_config();
    cfg.port = 70000;
    let err = cfg.validate().unwrap_err();
    match err {
        dyn_properties::Error::Validation { field_path, .. } => assert_eq!(field_path, "port"),
        _ => panic!("expected Validation error"),
    }
}

#[test]
fn string_too_short_fails() {
    let mut cfg = valid_config();
    cfg.host = "ab".to_string();
    assert!(cfg.validate().is_err());
}

#[test]
fn duration_out_of_range_fails() {
    let mut cfg = valid_config();
    cfg.connect_timeout = "1h".parse().unwrap();
    assert!(cfg.validate().is_err());
}

#[test]
fn option_bound_checked_only_when_some() {
    let mut cfg = valid_config();
    cfg.max_conns = None;
    assert!(cfg.validate().is_ok());

    cfg.max_conns = Some(1000);
    assert!(cfg.validate().is_err());
}

#[test]
fn nested_struct_validated_with_dot_joined_field_path() {
    let mut cfg = valid_config();
    cfg.pool.idle = 999;
    let err = cfg.validate().unwrap_err();
    match err {
        dyn_properties::Error::Validation { field_path, .. } => assert_eq!(field_path, "pool.idle"),
        _ => panic!("expected Validation error"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (Validate not implemented yet)**

Run: `cargo test -p dyn-properties --test validate`
Expected: FAIL to compile — `the trait bound DbConfig: Validate is not satisfied`.

- [ ] **Step 3: Implement the real `validate_gen::generate`**

Replace the entire contents of `dyn-properties-derive/src/validate_gen.rs`:

```rust
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
        (FieldKind::Duration, Some(Bound::DurationRange { min, max })) => {
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
    if let (FieldKind::Duration, Bound::DurationRange { min, max }) = (inner, bound) {
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
            static MIN: ::std::sync::LazyLock<dyn_properties::Duration> = ::std::sync::LazyLock::new(|| {
                <dyn_properties::Duration as ::std::str::FromStr>::from_str(#min)
                    .expect(&::std::format!("invalid #[duration_range] min literal on field `{}`", #field_name))
            });
            static MAX: ::std::sync::LazyLock<dyn_properties::Duration> = ::std::sync::LazyLock::new(|| {
                <dyn_properties::Duration as ::std::str::FromStr>::from_str(#max)
                    .expect(&::std::format!("invalid #[duration_range] max literal on field `{}`", #field_name))
            });
            if self.#ident < *MIN || self.#ident > *MAX {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("{} is out of range [{}, {}]", self.#ident, *MIN, *MAX),
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
            static MIN: ::std::sync::LazyLock<dyn_properties::Duration> = ::std::sync::LazyLock::new(|| {
                <dyn_properties::Duration as ::std::str::FromStr>::from_str(#min)
                    .expect(&::std::format!("invalid #[duration_range] min literal on field `{}`", #field_name))
            });
            static MAX: ::std::sync::LazyLock<dyn_properties::Duration> = ::std::sync::LazyLock::new(|| {
                <dyn_properties::Duration as ::std::str::FromStr>::from_str(#max)
                    .expect(&::std::format!("invalid #[duration_range] max literal on field `{}`", #field_name))
            });
            if *v < *MIN || *v > *MAX {
                return ::std::result::Result::Err(dyn_properties::Error::Validation {
                    field_path: #field_name.to_string(),
                    reason: ::std::format!("{} is out of range [{}, {}]", v, *MIN, *MAX),
                });
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --test validate`
Expected: PASS, all 7 tests green.

- [ ] **Step 5: Re-run the trybuild suite to confirm the new codegen didn't change the compile-fail diagnostics**

Run: `cargo test -p dyn-properties-derive --test trybuild`
Expected: PASS. If it fails because the emitted error text changed, re-run with `TRYBUILD=overwrite` and re-check the `.stderr` content (Task 6, Step 6) before committing the updated snapshot.

- [ ] **Step 6: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add dyn-properties-derive/src/validate_gen.rs tests/validate.rs
git commit -m "$(cat <<'EOF'
Generate real Validate impls from bound attributes

Covers numeric range, string length, duration range (via a lazily
parsed literal), Option<T> (checked only when Some), and recursive
nested-struct validation with dot-joined field paths.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `Default` codegen

**Files:**
- Modify: `dyn-properties-derive/src/default_gen.rs`
- Create: `tests/default.rs`

**Interfaces:**
- Consumes: `ParsedField` (Task 6), `dyn_properties::Duration::from_str` (Task 2).
- Produces: for every `#[derive(DynProperties)]` struct, a real `impl std::default::Default for StructName` — `#[default(v)]` fields use `v` (coerced via a `let`-bound `&str` for `String`/`Duration`, spliced directly for numeric/nested so normal Rust type-checking rejects a wrong-kind literal); unannotated fields fall back to `Default::default()` (which is `None` for `Option<T>` and the nested struct's own generated `Default` for nested fields).

- [ ] **Step 1: Write the failing integration tests**

Create `tests/default.rs`:

```rust
use dyn_properties::DynProperties;

#[derive(DynProperties)]
struct PoolConfig {
    #[range(min = 0, max = 50)]
    #[default(5)]
    idle: u32,

    #[range(min = 1, max = 20)]
    active: u32,
}

#[derive(DynProperties)]
struct DbConfig {
    #[len(min = 3, max = 64)]
    #[default("localhost")]
    host: String,

    #[range(min = 1, max = 65535)]
    #[default(5432)]
    port: u16,

    #[duration_range(min = "100ms", max = "30s")]
    #[default("5s")]
    connect_timeout: dyn_properties::Duration,

    #[range(min = 1, max = 100)]
    max_conns: u32,

    pool: PoolConfig,

    description: Option<String>,

    #[default("primary")]
    label: Option<String>,
}

#[test]
fn default_uses_attribute_values() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 5432);
    assert_eq!(*cfg.connect_timeout, std::time::Duration::from_secs(5));
}

#[test]
fn default_falls_back_to_type_default_without_attribute() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.max_conns, 0);
    assert_eq!(cfg.description, None);
}

#[test]
fn nested_struct_uses_its_own_generated_default() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.pool.idle, 5);
    assert_eq!(cfg.pool.active, 0);
}

#[test]
fn option_field_with_default_attribute_becomes_some() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.label, Some("primary".to_string()));
}
```

- [ ] **Step 2: Run the tests to verify they fail (Default not implemented yet)**

Run: `cargo test -p dyn-properties --test default`
Expected: FAIL to compile — `the trait bound DbConfig: Default is not satisfied`.

- [ ] **Step 3: Implement the real `default_gen::generate`**

Replace the entire contents of `dyn-properties-derive/src/default_gen.rs`:

```rust
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
```

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --test default`
Expected: PASS, all 4 tests green.

- [ ] **Step 5: Re-run the trybuild suite**

Run: `cargo test -p dyn-properties-derive --test trybuild`
Expected: PASS (unchanged — `default_gen` doesn't affect the compile-fail fixtures, which have no `#[default]` attributes).

- [ ] **Step 6: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add dyn-properties-derive/src/default_gen.rs tests/default.rs
git commit -m "$(cat <<'EOF'
Generate real Default impls from #[default] attributes

Fields without #[default] fall back to their type's own
Default::default() (None for Option<T>, the nested struct's own
generated Default for nested fields).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `Deserialize` codegen (default-overlay parsing)

**Files:**
- Modify: `dyn-properties-derive/src/deserialize_gen.rs`
- Create: `tests/deserialize.rs`

**Interfaces:**
- Consumes: `ParsedField` (Task 6), the generated `Default` impl (Task 8), `dyn_properties::exports::serde` (Task 1).
- Produces: for every `#[derive(DynProperties)]` struct, a real `impl<'de> dyn_properties::exports::serde::Deserialize<'de> for StructName` that deserializes into a private per-field-`Option`-wrapped helper struct, then overlays present fields onto `Self::default()` (`.unwrap_or(..)` for plain fields, `.or(..)` for fields that were already `Option<T>`, so the TOML file only needs to specify values that differ from the default, at any nesting depth).

- [ ] **Step 1: Write the failing integration tests**

Create `tests/deserialize.rs`:

```rust
use dyn_properties::DynProperties;

#[derive(DynProperties)]
struct PoolConfig {
    #[range(min = 0, max = 50)]
    #[default(5)]
    idle: u32,

    #[range(min = 1, max = 20)]
    #[default(2)]
    active: u32,
}

#[derive(DynProperties)]
struct DbConfig {
    #[len(min = 3, max = 64)]
    #[default("localhost")]
    host: String,

    #[range(min = 1, max = 65535)]
    #[default(5432)]
    port: u16,

    pool: PoolConfig,

    description: Option<String>,
}

#[test]
fn empty_toml_uses_all_defaults() {
    let cfg: DbConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 5432);
    assert_eq!(cfg.pool.idle, 5);
    assert_eq!(cfg.pool.active, 2);
    assert_eq!(cfg.description, None);
}

#[test]
fn overriding_top_level_field_keeps_other_defaults() {
    let cfg: DbConfig = toml::from_str("port = 9999").unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 9999);
}

#[test]
fn partial_nested_table_only_overrides_specified_subfield() {
    let cfg: DbConfig = toml::from_str("[pool]\nidle = 40").unwrap();
    assert_eq!(cfg.pool.idle, 40);
    assert_eq!(cfg.pool.active, 2);
}

#[test]
fn option_field_present_becomes_some() {
    let cfg: DbConfig = toml::from_str(r#"description = "primary db""#).unwrap();
    assert_eq!(cfg.description, Some("primary db".to_string()));
}

#[test]
fn option_field_absent_stays_none() {
    let cfg: DbConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.description, None);
}

#[test]
fn fully_specified_toml_overrides_everything() {
    let cfg: DbConfig = toml::from_str(
        r#"
        host = "db.internal"
        port = 6543

        [pool]
        idle = 10
        active = 4
        "#,
    )
    .unwrap();
    assert_eq!(cfg.host, "db.internal");
    assert_eq!(cfg.port, 6543);
    assert_eq!(cfg.pool.idle, 10);
    assert_eq!(cfg.pool.active, 4);
}
```

- [ ] **Step 2: Run the tests to verify they fail (Deserialize not implemented yet)**

Run: `cargo test -p dyn-properties --test deserialize`
Expected: FAIL to compile — `the trait bound DbConfig: serde::Deserialize<'_> is not satisfied`.

- [ ] **Step 3: Implement the real `deserialize_gen::generate`**

Replace the entire contents of `dyn-properties-derive/src/deserialize_gen.rs`:

```rust
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
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
```

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --test deserialize`
Expected: PASS, all 6 tests green.

- [ ] **Step 5: Re-run the trybuild suite**

Run: `cargo test -p dyn-properties-derive --test trybuild`
Expected: PASS.

- [ ] **Step 6: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS — this now exercises `Validate`, `Default`, and `Deserialize` together across `tests/smoke.rs`, `tests/validate.rs`, `tests/default.rs`, `tests/deserialize.rs`.

- [ ] **Step 7: Commit**

```bash
git add dyn-properties-derive/src/deserialize_gen.rs tests/deserialize.rs
git commit -m "$(cat <<'EOF'
Generate real Deserialize impls with default-overlay parsing

Each struct gets a private helper with every field wrapped in
Option (fields already Option<T> stay single-wrapped), then
overlays present TOML keys onto Self::default(). Nested derived
structs recurse automatically, so a partial [table] only overrides
its specified subfields.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: `PropertyWatcher<T>`

**Files:**
- Create: `src/watcher.rs`
- Modify: `src/lib.rs`
- Create: `tests/watcher.rs`

**Interfaces:**
- Consumes: `dyn_properties::{Error, Validate}` (Task 3), the generated `Default`/`Deserialize`/`Validate` impls (Tasks 7-9), `arc_swap::{ArcSwap, Guard}`, `tokio`.
- Produces: `dyn_properties::PropertyWatcher<T>` with `async fn start(path: impl Into<PathBuf>, interval: std::time::Duration) -> Result<Self, Error>` (requires `T: serde::de::DeserializeOwned + Validate + Default + Send + Sync + 'static`), `fn load(&self) -> arc_swap::Guard<Arc<T>>`, and a `Drop` impl that aborts the background task.

- [ ] **Step 1: Write the failing integration tests**

Create `tests/watcher.rs`:

```rust
use dyn_properties::{DynProperties, PropertyWatcher};
use std::io::Write;
use std::time::Duration;
use tracing_test::traced_test;

#[derive(DynProperties)]
struct AppConfig {
    #[range(min = 1, max = 65535)]
    #[default(8080)]
    port: u16,
}

#[tokio::test]
async fn start_loads_initial_values() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(watcher.load().port, 9000);
}

#[tokio::test]
async fn start_fails_on_invalid_initial_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 99999").unwrap();

    let result = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_secs(60)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn reload_picks_up_valid_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(watcher.load().port, 9000);

    std::fs::write(file.path(), "port = 9500").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9500);
}

#[tokio::test]
#[traced_test]
async fn reload_keeps_last_good_value_on_invalid_change_and_logs() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();

    std::fs::write(file.path(), "port = 99999").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload failed"));
}
```

- [ ] **Step 2: Run the tests to verify they fail (PropertyWatcher doesn't exist yet)**

Run: `cargo test -p dyn-properties --test watcher`
Expected: FAIL to compile — `unresolved import dyn_properties::PropertyWatcher`.

- [ ] **Step 3: Implement `PropertyWatcher<T>`**

Create `src/watcher.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use tokio::task::JoinHandle;

use crate::{Error, Validate};

pub struct PropertyWatcher<T> {
    inner: Arc<ArcSwap<T>>,
    handle: JoinHandle<()>,
}

impl<T> PropertyWatcher<T>
where
    T: serde::de::DeserializeOwned + Validate + Default + Send + Sync + 'static,
{
    pub async fn start(path: impl Into<PathBuf>, interval: std::time::Duration) -> Result<Self, Error> {
        let path = path.into();
        let initial = load_and_validate::<T>(&path).await?;
        let inner = Arc::new(ArcSwap::new(Arc::new(initial)));

        let watcher_inner = Arc::clone(&inner);
        let watch_path = path.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // first tick fires immediately; the initial load above already happened
            loop {
                ticker.tick().await;
                match load_and_validate::<T>(&watch_path).await {
                    Ok(value) => {
                        watcher_inner.store(Arc::new(value));
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %watch_path.display(),
                            error = %e,
                            "dyn-properties: reload failed, keeping previous value"
                        );
                    }
                }
            }
        });

        Ok(PropertyWatcher { inner, handle })
    }

    pub fn load(&self) -> Guard<Arc<T>> {
        self.inner.load()
    }
}

impl<T> Drop for PropertyWatcher<T> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn load_and_validate<T>(path: &Path) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned + Validate,
{
    let contents = tokio::fs::read_to_string(path).await?;
    let value: T = toml::from_str(&contents)?;
    value.validate()?;
    Ok(value)
}
```

- [ ] **Step 4: Wire the module into the crate root**

Add to `src/lib.rs`:

```rust
mod watcher;
pub use watcher::PropertyWatcher;
```

- [ ] **Step 5: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --test watcher`
Expected: PASS, all 4 tests green.

- [ ] **Step 6: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/watcher.rs src/lib.rs tests/watcher.rs
git commit -m "$(cat <<'EOF'
Add PropertyWatcher<T> for atomic polling reload

start() loads and validates once synchronously (failing fast on a
bad initial file), then spawns a tokio task that re-parses and
re-validates on each interval tick, publishing via ArcSwap only on
success and logging (tracing::warn!) and discarding on failure.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Crate-root documentation

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-10.
- Produces: a crate-level doc comment with a runnable example (full flow + the `Arc::clone`-for-`await` guidance) and a doctest demonstrating the recommended `T::default().validate().is_ok()` test pattern.

- [ ] **Step 1: Add the crate-level doc comment**

Insert at the very top of `src/lib.rs`, above the existing `mod`/`pub use` lines:

```rust
//! Reads a TOML file into a validated, hot-reloadable struct.
//!
//! ```no_run
//! use dyn_properties::{DynProperties, Duration, PropertyWatcher};
//! use std::sync::Arc;
//!
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[len(min = 3, max = 64)]
//!     #[default("localhost")]
//!     host: String,
//!
//!     #[range(min = 1, max = 65535)]
//!     #[default(8080)]
//!     port: u16,
//!
//!     #[duration_range(min = "100ms", max = "30s")]
//!     #[default("5s")]
//!     request_timeout: Duration,
//! }
//!
//! # async fn run() -> Result<(), dyn_properties::Error> {
//! let watcher = PropertyWatcher::<AppConfig>::start(
//!     "config.toml",
//!     std::time::Duration::from_secs(30),
//! ).await?;
//!
//! // Short-lived, same-thread read:
//! let port = watcher.load().port;
//!
//! // Crossing an `.await` or moving to another task/thread: clone the Arc
//! // out first. arc-swap documents that Guards use a bounded pool of
//! // fast thread-local slots and aren't meant to be held across yield
//! // points.
//! let cfg: Arc<AppConfig> = Arc::clone(&watcher.load());
//! some_async_fn(cfg).await;
//! # let _ = port;
//! # Ok(())
//! # }
//! # async fn some_async_fn(_cfg: Arc<AppConfig>) {}
//! ```
//!
//! ## Testing your config struct
//!
//! An out-of-bounds `#[default(..)]` is only caught by [`Validate`] the
//! first time it's actually constructed, rather than at compile time.
//! Add a test like this for any struct with defaults:
//!
//! ```
//! # use dyn_properties::{DynProperties, Validate};
//! #[derive(DynProperties)]
//! struct AppConfig {
//!     #[range(min = 1, max = 65535)]
//!     #[default(8080)]
//!     port: u16,
//! }
//!
//! assert!(AppConfig::default().validate().is_ok());
//! ```
```

- [ ] **Step 2: Run the doctests**

Run: `cargo test --doc -p dyn-properties`
Expected: PASS, both doctests compile (the first as `no_run`) and the second executes and passes.

- [ ] **Step 3: Run the full workspace test suite one last time**

Run: `cargo test --workspace`
Expected: PASS — this is the full suite: `dyn-properties-derive`'s unit tests (`attrs`, `type_kind`) and trybuild compile-fail tests, `dyn-properties`'s unit tests (`duration`, `error`) and integration tests (`smoke`, `validate`, `default`, `deserialize`, `watcher`), and the two doctests.

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs
git commit -m "$(cat <<'EOF'
Add crate-level documentation with a runnable example

Covers the full start()/load() flow, the Arc::clone-for-await
guidance from the Send/Sync design section, and the recommended
T::default().validate().is_ok() test pattern for defaults.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
