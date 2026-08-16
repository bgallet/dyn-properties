# dyn-properties: Pluggable Config Format

## Purpose

`dyn-properties` currently hardcodes TOML as the config file format:
`PropertyWatcher::load_and_validate` calls `toml::from_str` directly, and
`Error::TomlParse(toml::de::Error)` bakes TOML into the error type. This
spec generalizes the crate to support any text-or-byte-based
serde-compatible format — shipping TOML and JSON, and letting a caller
plug in their own (YAML, RON, whatever) without needing a change to this
crate — while also letting a build compile in only the format(s) it
actually uses.

## Context: why this is a small change

The derive macro's generated `Deserialize` impl (`deserialize_gen.rs`)
is already format-agnostic — it's a plain `serde::Deserialize` impl, not
TOML-specific. So `#[derive(DynProperties)]` itself needs **no changes**
for this spec. The TOML coupling is isolated to exactly three places:

1. `src/watcher.rs`: `toml::from_str(&contents)` in `load_and_validate`.
2. `src/error.rs`: the `Error::TomlParse(toml::de::Error)` variant and
   its `From` impl.
3. `Cargo.toml`: `toml = "0.8"` as an unconditional dependency.

## Goals

- A public `Format` trait, generic over the target type, that abstracts
  "turn bytes into `T`" — third parties can implement it for their own
  format without a PR against this crate.
- Ship `Toml` and `Json` implementations of `Format`, each gated behind
  its own Cargo feature, with **no default feature** — a consumer must
  explicitly enable `toml`, `json`, or both. A build enabling only one
  never pulls in the other format's dependency.
- `PropertyWatcher<T, F>` becomes generic over the format: which format
  is used is a compile-time, per-call-site choice
  (`PropertyWatcher::<AppConfig, Toml>::start(...)`), not runtime
  extension-sniffing.
- `Error` becomes format-agnostic (`Error::Parse(Box<dyn
  std::error::Error + Send + Sync>)`), so its shape doesn't change
  depending on which format features are compiled in.

## Non-goals

- Extension-based auto-detection (`.toml` → TOML, `.json` → JSON) —
  deliberately rejected in favor of the explicit type parameter; don't
  reintroduce this without a fresh design discussion.
- A default Cargo feature — this is a deliberate choice (see Cargo
  features section) even though it's a breaking change from today's
  zero-config `dyn-properties = "0.1"` behavior.
- Built-in YAML/RON/etc. support — the trait is designed to make these
  easy to add later (by us or by a third party), but this spec ships
  exactly TOML and JSON.
- Any change to the derive macro (`dyn-properties-derive`) — none is
  needed.

## The `Format` trait

```rust
pub trait Format {
    type Error: std::error::Error + Send + Sync + 'static;
    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error>;
}
```

- Stateless (no `&self`) — implementations are zero-sized marker types
  (`pub struct Toml;`, `pub struct Json;`), selected purely at the type
  level via `PropertyWatcher<T, F>`'s `F` parameter.
- Takes `&[u8]` rather than `&str`: both TOML and JSON are text formats
  and validate UTF-8 internally, but a byte-oriented signature means a
  future binary format (bincode, postcard, ...) wouldn't require
  changing the trait.
- `Self::Error: std::error::Error + Send + Sync + 'static` so it can be
  boxed into `Error::Parse` uniformly regardless of which format
  produced it.
- The trait itself is **not** feature-gated — it's always compiled, so
  a consumer building with `default-features = false` and neither
  `toml` nor `json` enabled can still use `dyn-properties` with their
  own `Format` implementation.

`Toml`/`Json` live in `src/format/toml.rs` / `src/format/json.rs`
(module `src/format.rs` holds the trait), each `#[cfg(feature =
"toml")]` / `#[cfg(feature = "json")]` respectively, and are re-exported
at the crate root: `dyn_properties::{Format, Toml, Json}` (`Toml`/`Json`
only exist when their feature is enabled).

## `PropertyWatcher<T, F>`

```rust
pub struct PropertyWatcher<T, F> {
    inner: Arc<ArcSwap<T>>,
    handle: JoinHandle<()>,
    _format: PhantomData<F>,
}

impl<T, F> PropertyWatcher<T, F>
where
    T: serde::de::DeserializeOwned + Validate + Default + Send + Sync + 'static,
    F: Format + Send + Sync + 'static,
{
    pub async fn start(path: impl Into<PathBuf>, interval: std::time::Duration) -> Result<Self, Error>;
    pub fn load(&self) -> arc_swap::Guard<Arc<T>>;
}
```

`F` has **no default type parameter**. A conditionally-valid default
(e.g. `F = Toml`) can't be made to compile correctly when the `toml`
feature is disabled, since Rust resolves a generic default unconditionally
at the struct definition site, not per call site — there's no way to
write "`F` defaults to `Toml` only if the `toml` feature happens to be
on, and to nothing otherwise" for a struct-level default. Every caller
therefore names the format explicitly:
`PropertyWatcher::<AppConfig, Toml>::start(...)`.

`load_and_validate` reads the file as bytes (`tokio::fs::read`, not
`read_to_string`) and calls `F::parse::<T>(&bytes)` instead of
`toml::from_str`.

This is a breaking change to every existing call site (adds a required
type parameter) — acceptable pre-release; the migration is mechanical
(add `, Toml` at each `PropertyWatcher::<T>` usage).

## `Error`

```rust
pub enum Error {
    Io(std::io::Error),
    Parse(Box<dyn std::error::Error + Send + Sync>),
    Validation { field_path: String, reason: String },
}
```

`TomlParse(toml::de::Error)` is replaced by the format-agnostic `Parse`
variant, constructed at the single call site in `load_and_validate`:

```rust
let value: T = F::parse(&bytes).map_err(|e| Error::Parse(Box::new(e)))?;
```

No blanket `From<E> for Error` is added (an `impl<E: std::error::Error
+ Send + Sync + 'static> From<E> for Error` would conflict with the
existing `From<std::io::Error> for Error` under Rust's coherence
rules, since `std::io::Error` also satisfies that bound) — the explicit
`.map_err(...)` at the one call site is simpler than working around
that.

`Display`/`source()` for `Parse` follow the same pattern as the other
variants (`write!(f, "parse error: {e}")`, `source()` returns
`Some(e.as_ref())`).

## Cargo features

```toml
[dependencies]
toml = { version = "0.8", optional = true }
serde_json = { version = "1", optional = true }

[features]
toml = ["dep:toml"]
json = ["dep:serde_json"]
```

Deliberately **no `default = [...]` entry** — every consumer must
explicitly enable `toml`, `json`, or both. This is a conscious
breaking change from today's zero-config behavior (`dyn-properties =
"0.1"` currently gives you TOML for free), chosen specifically so that
"compile the crate for a single deserialization format" is a real,
verified property (see Testing) rather than an aspiration undermined by
a default that always pulls in TOML anyway.

Enabling only `json` never pulls `toml` (or vice versa); enabling both
is fine for a binary that needs to read either. `dyn-properties-derive`
is entirely unaffected — no format-related features there.

## Testing strategy

- All standard dev-loop commands change from `cargo test --workspace`
  to `cargo test --workspace --all-features` (both formats enabled,
  matches today's full test coverage).
- New: `cargo test -p dyn-properties --no-default-features --features
  toml` and the mirror `--features json` each build and pass on their
  own — this is the actual proof that single-format builds work, not
  just a claim. (`--no-default-features` is technically redundant once
  there's no default feature, but keep it in the command for
  self-documentation and to stay correct if a default is ever added
  back.)
- New `tests/json.rs`, mirroring the existing TOML-based
  `tests/deserialize.rs`/`tests/default.rs` coverage at least for the
  default-overlay and bounds-checking behavior (doesn't need to
  duplicate every single existing TOML test case).
- A test proving a **caller-defined** `Format` impl works with
  `PropertyWatcher` — e.g. a trivial custom format in the test file
  itself — to prove the trait is genuinely usable outside this crate,
  not just by the two we ship.
- Existing tests (`tests/validate.rs`, `tests/default.rs`,
  `tests/deserialize.rs`, `tests/watcher.rs`, `tests/smoke.rs`) and the
  crate-level doc example in `src/lib.rs` are migrated to
  `PropertyWatcher::<T, Toml>` and gain `use dyn_properties::Toml;`
  (or similar) where needed.

## Migration notes

- Every `PropertyWatcher::<SomeConfig>::start(...)` in tests/docs
  becomes `PropertyWatcher::<SomeConfig, Toml>::start(...)`.
- `Error::TomlParse(_)` match arms (if any existed outside this crate)
  become `Error::Parse(_)`. Within this crate, the one test in
  `src/error.rs` that constructs `Error::TomlParse(...)` to prove
  `prefixed` leaves non-`Validation` variants alone is updated to
  construct `Error::Parse(Box::new(...))` instead — same behavior
  under test, different variant name.
