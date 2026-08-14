# dyn-properties: Design

## Purpose

A Rust library that reads a TOML file into a user-defined struct, validates
field values against optional bounds, and republishes the struct atomically
on a fixed interval so callers always see a fully-consistent snapshot of
configuration without blocking or partial updates.

## Goals

- Derive-macro-driven: callers annotate a plain struct, no manual parsing
  code.
- Field types: `String`, all standard integer widths, `f32`/`f64`, a
  `Duration` type parsed from strings like `"30s"`, `Option<T>` of any of
  the above, and nested structs that are themselves `#[derive(DynProperties)]`.
- Optional per-field bounds: string length, numeric range, duration range.
- Background reload on a fixed interval; a failed reload (I/O, parse, or
  bounds error) is logged and discarded — the previous good value stays
  live.
- Atomic, non-blocking reads via `ArcSwap`; readers never observe a
  torn/partial struct.

## Non-goals

- File-change-event watching (e.g. `notify`) — interval polling only.
- Partial/per-field merge on reload — a reload either fully replaces the
  struct or is fully rejected.
- Runtime-agnostic scheduling — the watcher owns a `tokio` task.

## Crate layout

Two-crate Cargo workspace:

- **`dyn-properties`** — the crate callers depend on. Public API:
  `PropertyWatcher<T>`, `Duration`, the `Validate` trait (implemented by
  generated code, not meant to be hand-implemented), `Error`, and a
  re-export of the derive macro (`pub use dyn_properties_derive::DynProperties;`).
  Runtime deps: `serde` (`derive` feature), `toml`, `arc-swap`, `tokio`
  (`rt`, `time` features), `tracing`.
- **`dyn-properties-derive`** — proc-macro crate implementing
  `#[derive(DynProperties)]` and the field attributes `#[range(min=..,
  max=..)]`, `#[len(min=.., max=..)]`, `#[duration_range(min=.., max=..)]`.
  Deps: `syn`, `quote`, `proc-macro2`.

## Field types & attributes

Callers write a plain struct with two derives — `serde::Deserialize` for
structural parsing, `DynProperties` for validation:

```rust
use dyn_properties::{DynProperties, Duration};

#[derive(DynProperties, serde::Deserialize)]
pub struct DbConfig {
    #[len(min = 3, max = 64)]
    pub host: String,

    #[range(min = 1, max = 65535)]
    pub port: u16,

    #[duration_range(min = "100ms", max = "30s")]
    pub connect_timeout: Duration,

    #[range(min = 1, max = 100)]
    pub max_conns: u32,

    pub pool: PoolConfig,              // nested struct, validated recursively
    pub description: Option<String>,   // optional, no bounds required
}

#[derive(DynProperties, serde::Deserialize)]
pub struct PoolConfig {
    #[range(min = 0, max = 50)]
    pub idle: u32,
}
```

Rules:

- A field with no bounds attribute is not checked (structurally parsed
  only). Nested-struct fields are always recursed into regardless of
  whether they carry an attribute.
- `#[range]` is valid on integer/float fields, `#[len]` on `String`,
  `#[duration_range]` on `Duration`. Using an attribute on a field of the
  wrong kind is a **compile-time error** raised by the macro via
  `syn`/`quote` diagnostics.
- `Option<T>` bounds apply only when the value is `Some`; `None` always
  passes validation.
- `Duration` bound literals (e.g. `"100ms"`) are parsed once, lazily
  (`std::sync::LazyLock`), the first time validation runs — not
  re-parsed on every reload.

### `Duration` type

`dyn_properties::Duration` is a newtype: `pub struct Duration(std::time::Duration);`
implementing `Deref<Target = std::time::Duration>` and `Display`, with a
hand-written `serde::Deserialize` (in `dyn-properties`, not
macro-generated) that parses `"<digits><unit>"` where `unit` is one of
`ms`, `s`, `m`, `h`, `d`. Invalid suffix, missing digits, or negative
values are deserialize errors.

## Parsing & validation pipeline (approach A)

Two separate passes, each independently testable:

1. **Parse**: `toml::from_str::<T>(contents)` via the caller's
   `serde::Deserialize` derive. Handles nested tables and `Option<T>` for
   free. Failure → `Error::TomlParse`.
2. **Validate**: the macro-generated `Validate::validate(&self) ->
   Result<(), Error>` walks each bounded field, comparing against the
   literal bounds from its attribute, and recurses into nested structs'
   own `validate()`. First failure short-circuits with
   `Error::Validation { field_path, reason }`, where `field_path` is
   dot-joined (e.g. `db.pool.idle`).

Both passes must succeed for a load (initial or reload) to take effect.

## `PropertyWatcher<T>` API

```rust
pub struct PropertyWatcher<T> {
    inner: Arc<ArcSwap<T>>,
    handle: tokio::task::JoinHandle<()>,
    // path, interval retained for Drop/diagnostics as needed
}

impl<T> PropertyWatcher<T>
where
    T: serde::de::DeserializeOwned + Validate + Send + Sync + 'static,
{
    pub async fn start(path: impl Into<std::path::PathBuf>, interval: std::time::Duration)
        -> Result<Self, dyn_properties::Error>;

    pub fn load(&self) -> arc_swap::Guard<Arc<T>>;
}

impl<T> Drop for PropertyWatcher<T> {
    fn drop(&mut self) { self.handle.abort(); }
}
```

- **Startup**: `start()` runs the parse+validate pipeline synchronously
  once. On failure, returns `Err` immediately (no watcher is created, no
  background task is spawned) — the caller decides whether to abort the
  process. On success, wraps the value in `Arc`, publishes it to a new
  `ArcSwap`, spawns a `tokio::time::interval`-driven task, and returns the
  watcher.
- **Reload tick**: same parse+validate pipeline. Success →
  `arc_swap.store(Arc::new(new_value))`, a single atomic pointer swap —
  any in-flight `Guard` keeps seeing a fully-consistent old or new value,
  never a mix. Failure → tick is skipped, previous `Arc` stays live,
  `tracing::warn!` logs the error including field path/cause.
- **Reading**: `watcher.load()` returns a `Guard<Arc<T>>` (derefs to
  `&T`) for an immediate, short-lived, same-thread read.
- **Shutdown**: dropping the `PropertyWatcher` aborts the background task.

### Send/Sync

`PropertyWatcher<T>` requires `T: Send + Sync + 'static` (declared on the
impl block). Given that bound, `Arc<ArcSwap<T>>` is `Send + Sync`
automatically (same rule as `Arc<T>`), and since the watcher's other
fields (`JoinHandle`, path/interval) are `Send + Sync` unconditionally,
the whole `PropertyWatcher<T>` is `Send + Sync` via ordinary Rust
auto-trait propagation — no `unsafe impl` anywhere in the library. Every
field type the macro accepts (`String`, numeric primitives, `Duration`,
`Option<T>`, nested derived structs) is `Send + Sync` on its own, so this
bound is satisfied for free in practice.

`Guard<Arc<T>>` (from `.load()`) is `Send`/`Sync` under the same
condition (confirmed against arc-swap's implementation). However,
arc-swap documents that guards use a bounded pool of fast thread-local
slots (currently 8) and are **not intended to be stored in data
structures or held across async yield points** — holding too many live
guards, or holding one across an `.await`, degrades to a slower fallback
path. The documented pattern for anything that needs to cross an
`.await` or move to another task/thread is to clone the owned `Arc<T>`
out immediately: `let cfg: Arc<T> = Arc::clone(&watcher.load());`. This
is called out explicitly in the crate's doc example.

## Error type

```rust
pub enum Error {
    Io(std::io::Error),
    TomlParse(toml::de::Error),
    Validation { field_path: String, reason: String },
}
```

Reload failures are surfaced via `tracing::warn!` only (no callback
mechanism) — matches the "log by default" decision; a callback can be
added later without a breaking change if needed.

## Testing strategy

- **`dyn-properties-derive`**: `trybuild` tests for compile-time errors
  (e.g. `#[range]` on a `String` field). Unit tests asserting generated
  `Validate` logic against hand-written structs: valid values pass,
  boundary values (exactly `min`/`max`) pass, one-over/one-under fail
  with the correct `field_path` in the error.
- **`dyn-properties`**: unit tests for `Duration` parsing (all five
  units, invalid suffix, missing digits, negative values). Integration
  tests using a temp file: `start()` a watcher and assert initial
  values; rewrite the file with valid content and assert the watcher
  reloads after the interval elapses (short interval, e.g. 50ms, to keep
  tests fast); rewrite with invalid content and assert the watcher keeps
  the last-good value and a warning is logged.
- A crate-root doc example showing the full flow: struct definition →
  `start()` → `load()` → the `Arc::clone`-for-`await` pattern.

## Open questions / follow-ups (explicitly deferred)

- Optional error callback in addition to `tracing` logging — deferred,
  can be added non-breaking later.
- File-change-event-based reload in addition to polling — deferred.
- Collections (`Vec<T>`, maps) as field types — not covered by this
  spec; add if a concrete need arises.
