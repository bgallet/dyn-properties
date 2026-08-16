# Pluggable Config Format Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize `dyn-properties` beyond hardcoded TOML: a public `Format` trait (implementable by third parties), `Toml`/`Json` shipped behind their own Cargo features with no default, `PropertyWatcher<T, F>` generic over format, and a format-agnostic `Error::Parse`.

**Architecture:** A stateless `Format` trait (`fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error>`) lives unconditionally in `src/format.rs`. `Toml`/`Json` are zero-sized marker types implementing it, each in their own file under `src/format/`, each gated behind its own Cargo feature (`toml`, `json`) with neither enabled by default. `PropertyWatcher<T, F>` gains a second type parameter selecting the format at every call site (no default — a feature-conditional default can't be expressed correctly). `Error::TomlParse(toml::de::Error)` becomes `Error::Parse(Box<dyn std::error::Error + Send + Sync>)`. The derive macro (`dyn-properties-derive`) needs **no changes** — its generated `Deserialize` impl is already plain, format-agnostic `serde`.

**Tech Stack:** Rust, `serde` (already a dependency), `toml` (now optional), `serde_json` (new, optional).

**Spec:** `docs/superpowers/specs/2026-08-16-pluggable-format.md`

## Global Constraints

- No default Cargo feature — every consumer must explicitly enable `toml`, `json`, or both. Enabling only one never pulls in the other format's dependency.
- The `Format` trait itself is never feature-gated — a caller can implement their own format and use `PropertyWatcher` with zero built-in formats enabled.
- `PropertyWatcher<T, F>` has no default type parameter for `F` — every call site names the format explicitly (e.g. `PropertyWatcher::<AppConfig, Toml>::start(...)`).
- `Error::Parse(Box<dyn std::error::Error + Send + Sync>)` replaces `Error::TomlParse` — one `Error` shape regardless of which format features are compiled in. No blanket `From<E> for Error` (would conflict with the existing `From<std::io::Error>` impl under coherence rules) — construct `Error::Parse` explicitly at the one call site that needs it.
- No extension-based auto-detection (`.toml` → TOML, `.json` → JSON) — explicitly out of scope; don't reintroduce without a fresh design discussion.
- No changes to `dyn-properties-derive` — none are needed for this plan.
- Because there's no default feature, every build/test command in this plan passes explicit `--features ...` (or `--all-features`) — a bare `cargo build`/`cargo test` will not compile once Task 1 lands, until Task 3 finishes decoupling `src/watcher.rs` from a hardcoded `toml` dependency. This is expected mid-plan, not a bug to chase.

---

## Task 1: `Format` trait and `Toml`

**Files:**
- Modify: `Cargo.toml`
- Create: `src/format.rs`
- Create: `src/format/toml_format.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: nothing new (pure addition).
- Produces: `dyn_properties::Format` (`trait { type Error: std::error::Error + Send + Sync + 'static; fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error>; }`), `dyn_properties::Toml` (zero-sized, `#[cfg(feature = "toml")]`, `impl Format for Toml { type Error = toml::de::Error; ... }`).

- [ ] **Step 1: Make `toml` an optional dependency and add the `toml` feature**

Edit `Cargo.toml`. Change:

```toml
toml = "0.8"
```

to:

```toml
toml = { version = "0.8", optional = true }
```

Add a new `[features]` section (place it after `[dependencies]`, before `[dev-dependencies]`):

```toml
[features]
toml = ["dep:toml"]
```

Add a new `[[test]]` section gating the existing TOML-specific integration test (place it after `[dev-dependencies]`, before `[workspace]`):

```toml
[[test]]
name = "deserialize"
required-features = ["toml"]
```

- [ ] **Step 2: Confirm the crate no longer builds without the `toml` feature (expected — this is the point of Step 1)**

Run: `cargo build -p dyn-properties`
Expected: FAILS — `unresolved import toml` (or similar) from `src/watcher.rs` and `src/error.rs`, which still reference the `toml` crate unconditionally. This is expected at this point in the plan; both are fixed in later tasks.

- [ ] **Step 3: Write the failing test for `Format`/`Toml`**

Create `src/format/toml_format.rs`:

```rust
use serde::de::Error as _;

use crate::Format;

/// Parses config files as TOML. Requires the `toml` Cargo feature.
pub struct Toml;

impl Format for Toml {
    type Error = toml::de::Error;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        let text = std::str::from_utf8(bytes).map_err(toml::de::Error::custom)?;
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Validate;
    use dyn_properties_derive::DynProperties;

    #[derive(DynProperties)]
    struct Cfg {
        #[range(min = 1, max = 100)]
        #[default(10)]
        count: u32,
    }

    #[test]
    fn parses_toml_bytes() {
        let cfg: Cfg = Toml::parse(b"count = 42").unwrap();
        assert_eq!(cfg.count, 42);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_toml() {
        let result: Result<Cfg, _> = Toml::parse(b"not valid = [");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Create the `Format` trait and wire the module tree**

Create `src/format.rs`:

```rust
/// Abstracts "parse bytes into `T`" so [`PropertyWatcher`](crate::PropertyWatcher) isn't
/// tied to one config file format. Implementations are typically zero-sized marker types
/// selected at the type level — e.g. this crate's own [`Toml`](crate::Toml) — and the
/// derive macro's generated `serde::Deserialize` impl works with any of them unchanged,
/// since it's already plain, format-agnostic `serde`.
///
/// Implement this for your own format (YAML, RON, ...) to use it with `PropertyWatcher`
/// without needing a change to this crate.
pub trait Format {
    /// The error type produced when `bytes` isn't valid for this format, or doesn't match
    /// `T`'s shape.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parses `bytes` into `T`.
    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error>;
}

#[cfg(feature = "toml")]
mod toml_format;
#[cfg(feature = "toml")]
pub use toml_format::Toml;
```

- [ ] **Step 5: Wire the module into the crate root**

Add to `src/lib.rs`, after the `mod duration; ...` block and before `mod error;`:

```rust
mod format;
pub use format::Format;
#[cfg(feature = "toml")]
pub use format::Toml;
```

- [ ] **Step 6: Run the new tests to confirm they pass**

Run: `cargo test -p dyn-properties --features toml --lib format::`
Expected: FAIL to compile at this point — `src/watcher.rs` and `src/error.rs` still reference the now-optional `toml` crate unconditionally (same failure as Step 2), so the whole crate can't build yet even with `--features toml` enabled, because those two files don't gate their `toml` usage behind the feature at all. This is expected — those files are fixed in Tasks 2-3. Confirm the *compile error* is specifically about `src/watcher.rs`/`src/error.rs`, not about anything in `src/format.rs` or `src/format/toml_format.rs` — if the error is instead in the new format code, that's a real bug to fix now.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/format.rs src/format/toml_format.rs src/lib.rs
git commit -m "$(cat <<'EOF'
Add Format trait and Toml implementation

toml becomes an optional dependency behind a new `toml` Cargo
feature. The crate doesn't fully build yet (src/watcher.rs and
src/error.rs still reference toml unconditionally) — fixed in the
next two tasks.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Format-agnostic `Error`

**Files:**
- Modify: `src/error.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `dyn_properties::Error` with `Parse(Box<dyn std::error::Error + Send + Sync>)` replacing `TomlParse(toml::de::Error)`. `prefixed`, `Display`, `std::error::Error`, and `From<std::io::Error>` keep their existing behavior/signatures.

- [ ] **Step 1: Update the existing test to use `Parse` instead of `TomlParse`**

In `src/error.rs`, replace the `prefixed_leaves_non_validation_variants_unchanged` test:

```rust
    #[test]
    fn prefixed_leaves_non_validation_variants_unchanged() {
        let parse_err = toml::from_str::<toml::Value>("not valid = [").unwrap_err();
        let err = Error::TomlParse(parse_err);
        let prefixed = err.prefixed("pool");
        assert!(matches!(prefixed, Error::TomlParse(_)));
    }
```

with:

```rust
    #[test]
    fn prefixed_leaves_non_validation_variants_unchanged() {
        let parse_err = "abc".parse::<i32>().unwrap_err();
        let err = Error::Parse(Box::new(parse_err));
        let prefixed = err.prefixed("pool");
        assert!(matches!(prefixed, Error::Parse(_)));
    }
```

(This drops `src/error.rs`'s only remaining reference to the `toml` crate — the test now builds its "some parse error" from `std::num::ParseIntError`, a always-available standard-library error, so this file no longer needs `toml` at all, matching the format-agnostic goal.)

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p dyn-properties --features toml --lib error::`
Expected: FAIL to compile — `no variant named Parse found for enum Error`.

- [ ] **Step 3: Replace `TomlParse` with `Parse` throughout the enum and its impls**

In `src/error.rs`, replace the whole file's non-test content with:

```rust
use std::fmt;

/// Everything that can go wrong loading and validating a config file: reading it,
/// parsing it (in whichever [`Format`](crate::Format) was selected), or checking it
/// against `#[range]`/`#[len]` bounds.
#[derive(Debug)]
pub enum Error {
    /// The config file could not be read (e.g. it doesn't exist or isn't readable).
    Io(std::io::Error),
    /// The file's contents didn't parse under the selected format, or didn't match the
    /// target struct's shape.
    Parse(Box<dyn std::error::Error + Send + Sync>),
    /// The file parsed fine but a field violated its declared bound.
    Validation {
        /// Dot-separated path to the offending field, e.g. `"pool.idle_timeout"` for a
        /// nested struct.
        field_path: String,
        /// Human-readable description of why the value is out of bounds.
        reason: String,
    },
}

impl Error {
    /// Prepends `parent_field` to a [`Error::Validation`]'s `field_path`, turning e.g.
    /// `"idle_timeout"` into `"pool.idle_timeout"` when a nested struct's validation
    /// error bubbles up through its parent. Non-`Validation` variants pass through
    /// unchanged. Used by the derive macro's generated `Validate` impls for nested
    /// struct fields; not typically called directly.
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
            Error::Parse(e) => write!(f, "parse error: {e}"),
            Error::Validation { field_path, reason } => write!(f, "{field_path}: {reason}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Parse(e) => Some(e.as_ref()),
            Error::Validation { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
```

(Note: the old `impl From<toml::de::Error> for Error` is dropped entirely — `Error::Parse` is now constructed explicitly where needed, in `src/watcher.rs` in Task 3.)

- [ ] **Step 4: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --features toml --lib error::`
Expected: still FAILS to compile — `src/watcher.rs` still references `toml` unconditionally and still constructs `Error::TomlParse`. This is expected; fixed in Task 3. Confirm the compile error has moved entirely into `src/watcher.rs` (no errors left pointing at `src/error.rs`) — that's the signal this task's own change is correct.

- [ ] **Step 5: Commit**

```bash
git add src/error.rs
git commit -m "$(cat <<'EOF'
Make Error format-agnostic: Parse(Box<dyn Error>) replaces TomlParse

One Error shape regardless of which format features are compiled
in. src/watcher.rs still references TomlParse/toml unconditionally
at this point — fixed in the next task.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `PropertyWatcher<T, F>`

**Files:**
- Modify: `src/watcher.rs`
- Modify: `Cargo.toml`
- Modify: `tests/watcher.rs`
- Create: `tests/custom_format.rs`

**Interfaces:**
- Consumes: `dyn_properties::{Format, Toml}` (Task 1), `dyn_properties::Error::Parse` (Task 2).
- Produces: `dyn_properties::PropertyWatcher<T, F>` — `start(path, interval) -> Result<Self, Error>` now requires `F: Format + Send + Sync + 'static` in addition to the existing bounds on `T`; `load()` unchanged in shape.

- [ ] **Step 1: Rewrite `src/watcher.rs` to be generic over the format**

Replace the entire contents of `src/watcher.rs`:

```rust
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use tokio::task::JoinHandle;

use crate::{Error, Format, Validate};

/// A config file loaded once and then polled on a background task, exposing the latest
/// successfully-parsed-and-validated value via [`load`](Self::load).
///
/// `F` selects the file format (e.g. [`Toml`](crate::Toml), [`Json`](crate::Json), or a
/// caller-defined [`Format`] implementation) — see [`Format`] for why this is a type
/// parameter rather than runtime extension-sniffing.
///
/// A failed reload (I/O error, parse error, or a `Validate` bound violation) is logged
/// via `tracing` and discarded, leaving the previously-loaded value in place. Dropping
/// the watcher stops the background polling task.
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
    /// Loads and validates `path` once, then spawns a background task that re-loads it
    /// every `interval`, replacing the stored value on success and keeping the previous
    /// one (with a logged warning) on failure.
    ///
    /// Returns an error if the initial load fails; the background task is only spawned
    /// once the initial load succeeds.
    pub async fn start(path: impl Into<PathBuf>, interval: std::time::Duration) -> Result<Self, Error> {
        let path = path.into();
        let initial = load_and_validate::<T, F>(&path).await?;
        let inner = Arc::new(ArcSwap::new(Arc::new(initial)));

        let watcher_inner = Arc::clone(&inner);
        let watch_path = path.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // first tick fires immediately; the initial load above already happened
            loop {
                ticker.tick().await;
                // Run each tick's load-and-validate in its own task so a panic inside it
                // (e.g. a malformed #[range] literal only forced once a
                // previously-`None` Option<Duration> field first becomes `Some`) is
                // caught as a `JoinError` here instead of unwinding this loop's task and
                // silently ending all future reloads.
                let tick_path = watch_path.clone();
                match tokio::spawn(async move { load_and_validate::<T, F>(&tick_path).await }).await {
                    Ok(Ok(value)) => {
                        watcher_inner.store(Arc::new(value));
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            path = %watch_path.display(),
                            error = %e,
                            "dyn-properties: reload failed, keeping previous value"
                        );
                    }
                    Err(join_err) => {
                        let panic_msg = join_err
                            .try_into_panic()
                            .map(|payload| panic_message(&payload))
                            .unwrap_or_else(|_| "reload task was cancelled".to_string());
                        tracing::error!(
                            path = %watch_path.display(),
                            panic = %panic_msg,
                            "dyn-properties: reload tick panicked, keeping previous value"
                        );
                    }
                }
            }
        });

        Ok(PropertyWatcher {
            inner,
            handle,
            _format: PhantomData,
        })
    }

    /// Returns a guard giving read access to the current value.
    ///
    /// The `Guard` is meant to be used and dropped quickly, on the same thread that
    /// obtained it: `arc-swap` backs it with a bounded pool of fast thread-local slots
    /// that aren't meant to be held across `.await` points or moved across threads. To
    /// carry the value across an `.await` or into another task/thread, clone the `Arc`
    /// out first: `Arc::clone(&watcher.load())`.
    pub fn load(&self) -> Guard<Arc<T>> {
        self.inner.load()
    }
}

/// Aborts the background reload task; the file is no longer polled once the watcher is
/// dropped.
impl<T, F> Drop for PropertyWatcher<T, F> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn load_and_validate<T, F>(path: &Path) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned + Validate,
    F: Format,
{
    let bytes = tokio::fs::read(path).await?;
    let value: T = F::parse(&bytes).map_err(|e| Error::Parse(Box::new(e)))?;
    value.validate()?;
    Ok(value)
}

/// Best-effort extraction of a human-readable message from a caught panic payload.
///
/// Takes `&Box<dyn Any + Send>` (not `&(dyn Any + Send)`) deliberately: coercing the
/// `Box` reference down to a bare trait-object reference before calling
/// `downcast_ref` breaks the downcast (it always returns `None`, even though
/// `TypeId` comparison succeeds). Calling `downcast_ref` directly on `&Box<..>`
/// via auto-deref at the call site works correctly.
#[allow(clippy::borrowed_box)]
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}
```

- [ ] **Step 2: Gate the `watcher` integration test behind the `toml` feature**

Add to `Cargo.toml`, in the same area as the `deserialize` `[[test]]` block added in Task 1:

```toml
[[test]]
name = "watcher"
required-features = ["toml"]
```

- [ ] **Step 3: Migrate `tests/watcher.rs` to the new generic API**

In `tests/watcher.rs`, change the import line:

```rust
use dyn_properties::{DynProperties, PropertyWatcher};
```

to:

```rust
use dyn_properties::{DynProperties, PropertyWatcher, Toml};
```

Then change every `PropertyWatcher::<AppConfig>::start(` to `PropertyWatcher::<AppConfig, Toml>::start(` (4 occurrences: in `start_loads_initial_values`, `start_fails_on_invalid_initial_file`, `reload_picks_up_valid_changes`, `reload_keeps_last_good_value_on_invalid_change_and_logs`), and `PropertyWatcher::<PanicProneConfig>::start(` to `PropertyWatcher::<PanicProneConfig, Toml>::start(` (1 occurrence, in `reload_survives_a_panic_inside_a_tick`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dyn-properties --features toml`
Expected: PASS — this is the first point in the plan where the whole crate (with `toml` enabled) builds and tests cleanly again. All prior tests (`lib` unit tests including `format::toml_format::tests`, `error::tests`, `tests/smoke.rs`, `tests/validate.rs`, `tests/default.rs`, `tests/deserialize.rs`, `tests/watcher.rs`) should pass.

- [ ] **Step 5: Add a test proving `Format` is usable by a caller-defined implementation, not just `Toml`/`Json`**

Create `tests/custom_format.rs`:

```rust
use dyn_properties::{DynProperties, Format, PropertyWatcher};
use serde::de::value::{Error as ValueError, MapDeserializer};
use serde::Deserialize;
use std::io::Write;
use std::time::Duration;

/// A trivial caller-defined format, proving `Format` is genuinely pluggable: not TOML or
/// JSON, no Cargo feature gate, defined entirely in this test file using only `serde`'s
/// own `de::value` helpers (part of the base `serde` crate — no extra dependency, and
/// this file has no `required-features` entry in Cargo.toml, so it runs in every feature
/// combination, including with neither `toml` nor `json` enabled).
///
/// "Parses" a bare decimal number into `{ count: <that number> }`.
struct PlainNumber;

impl Format for PlainNumber {
    type Error = ValueError;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        let text = std::str::from_utf8(bytes).map_err(|e| serde::de::Error::custom(e.to_string()))?;
        let value: u64 = text
            .trim()
            .parse()
            .map_err(|_| serde::de::Error::custom(format!("not a plain decimal number: {text}")))?;
        let pairs = vec![("count", value)];
        T::deserialize(MapDeserializer::new(pairs.into_iter()))
    }
}

#[derive(DynProperties)]
struct Cfg {
    #[range(min = 0, max = 1000)]
    #[default(0)]
    count: u32,
}

#[tokio::test]
async fn watcher_works_with_a_caller_defined_format() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "42").unwrap();

    let watcher = PropertyWatcher::<Cfg, PlainNumber>::start(file.path(), Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(watcher.load().count, 42);
}
```

If `serde::de::value::MapDeserializer::new` doesn't accept a plain iterator of `(&str, u64)` tuples in the `serde` version this workspace resolves to (check with `cargo doc -p serde --open`, or docs.rs for the exact resolved version — run `cargo tree -p dyn-properties -i serde` to find it), adapt the body of `PlainNumber::parse` to whatever the current `serde::de::value` API requires to build a `T` from a single `"count" -> u64` pair — the test's *intent* (a fully self-contained, non-built-in `Format` impl proving the trait is usable outside this crate) is what must be preserved, not this exact snippet.

- [ ] **Step 6: Run the new test to verify it passes**

Run: `cargo test -p dyn-properties --test custom_format`
Expected: PASS. Note this command has **no `--features` flag** — confirm it still passes, since `tests/custom_format.rs` has no `required-features` entry and doesn't reference `Toml`/`Json` at all.

- [ ] **Step 7: Run the full workspace suite**

Run: `cargo test -p dyn-properties --features toml`
Expected: PASS (includes `tests/custom_format.rs` again, alongside everything else).

- [ ] **Step 8: Commit**

```bash
git add src/watcher.rs Cargo.toml tests/watcher.rs tests/custom_format.rs
git commit -m "$(cat <<'EOF'
Make PropertyWatcher<T, F> generic over the config format

load_and_validate now reads bytes and calls F::parse instead of
hardcoding toml::from_str. Existing tests migrate to
PropertyWatcher::<T, Toml>; a new caller-defined Format (no Cargo
feature, no built-in format crate) proves the trait is genuinely
pluggable, not just usable by Toml/Json.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `Json`

**Files:**
- Modify: `Cargo.toml`
- Create: `src/format/json_format.rs`
- Modify: `src/format.rs`
- Modify: `src/lib.rs`
- Create: `tests/json.rs`

**Interfaces:**
- Consumes: `dyn_properties::Format` (Task 1).
- Produces: `dyn_properties::Json` (zero-sized, `#[cfg(feature = "json")]`, `impl Format for Json { type Error = serde_json::Error; ... }`).

- [ ] **Step 1: Add `serde_json` as an optional dependency and the `json` feature**

Edit `Cargo.toml`. In `[dependencies]`, add (near `toml`):

```toml
serde_json = { version = "1", optional = true }
```

In `[features]`, add:

```toml
json = ["dep:serde_json"]
```

Add a new `[[test]]` block, alongside the `deserialize`/`watcher` ones from Tasks 1 and 3:

```toml
[[test]]
name = "json"
required-features = ["json"]
```

- [ ] **Step 2: Write the failing tests for `Json`**

Create `src/format/json_format.rs`:

```rust
use crate::Format;

/// Parses config files as JSON. Requires the `json` Cargo feature.
pub struct Json;

impl Format for Json {
    type Error = serde_json::Error;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Validate;
    use dyn_properties_derive::DynProperties;

    #[derive(DynProperties)]
    struct Cfg {
        #[range(min = 1, max = 100)]
        #[default(10)]
        count: u32,
    }

    #[test]
    fn parses_json_bytes() {
        let cfg: Cfg = Json::parse(br#"{"count": 42}"#).unwrap();
        assert_eq!(cfg.count, 42);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_json() {
        let result: Result<Cfg, _> = Json::parse(b"not valid json");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail to compile**

Run: `cargo test -p dyn-properties --features json --lib format::`
Expected: FAIL to compile — `src/format.rs` doesn't declare the `json_format` module yet.

- [ ] **Step 4: Wire `Json` into `src/format.rs`**

Add to `src/format.rs`, after the existing `#[cfg(feature = "toml")] ...` block:

```rust
#[cfg(feature = "json")]
mod json_format;
#[cfg(feature = "json")]
pub use json_format::Json;
```

- [ ] **Step 5: Re-export `Json` at the crate root**

Add to `src/lib.rs`, next to the existing `#[cfg(feature = "toml")] pub use format::Toml;` line:

```rust
#[cfg(feature = "json")]
pub use format::Json;
```

- [ ] **Step 6: Run the tests again to verify they pass**

Run: `cargo test -p dyn-properties --features json --lib format::`
Expected: PASS, all `format::json_format::tests` green.

- [ ] **Step 7: Run the full crate test suite with ONLY the `json` feature — this is the key proof of this whole plan**

Run: `cargo test -p dyn-properties --no-default-features --features json`
Expected: PASS. `tests/deserialize.rs` and `tests/watcher.rs` (both `required-features = ["toml"]`) are silently skipped — confirm the test output says something like "0 tests" or lists them as skipped, not an error. `tests/smoke.rs`, `tests/validate.rs`, `tests/default.rs`, `tests/custom_format.rs`, and the new `tests/json.rs` all run and pass. This proves the crate genuinely builds and runs with **zero** dependency on the `toml` crate when only `json` is enabled.

- [ ] **Step 8: Add the JSON integration test suite, mirroring `tests/deserialize.rs`'s TOML coverage**

Create `tests/json.rs`:

```rust
use dyn_properties::{DynProperties, Json};

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
fn empty_json_object_uses_all_defaults() {
    let cfg: DbConfig = Json::parse(b"{}").unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 5432);
    assert_eq!(cfg.pool.idle, 5);
    assert_eq!(cfg.pool.active, 2);
    assert_eq!(cfg.description, None);
}

#[test]
fn overriding_top_level_field_keeps_other_defaults() {
    let cfg: DbConfig = Json::parse(br#"{"port": 9999}"#).unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 9999);
}

#[test]
fn partial_nested_object_only_overrides_specified_subfield() {
    let cfg: DbConfig = Json::parse(br#"{"pool": {"idle": 40}}"#).unwrap();
    assert_eq!(cfg.pool.idle, 40);
    assert_eq!(cfg.pool.active, 2);
}

#[test]
fn option_field_present_becomes_some() {
    let cfg: DbConfig = Json::parse(br#"{"description": "primary db"}"#).unwrap();
    assert_eq!(cfg.description, Some("primary db".to_string()));
}

#[test]
fn option_field_absent_stays_none() {
    let cfg: DbConfig = Json::parse(b"{}").unwrap();
    assert_eq!(cfg.description, None);
}

#[test]
fn fully_specified_json_overrides_everything() {
    let cfg: DbConfig = Json::parse(
        br#"{
            "host": "db.internal",
            "port": 6543,
            "pool": { "idle": 10, "active": 4 }
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.host, "db.internal");
    assert_eq!(cfg.port, 6543);
    assert_eq!(cfg.pool.idle, 10);
    assert_eq!(cfg.pool.active, 4);
}
```

- [ ] **Step 9: Run the new integration suite**

Run: `cargo test -p dyn-properties --no-default-features --features json --test json`
Expected: PASS, all 6 tests green.

- [ ] **Step 10: Run the full workspace suite with both formats enabled**

Run: `cargo test -p dyn-properties --all-features`
Expected: PASS — everything from every prior task, plus `tests/json.rs`, all together.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml src/format.rs src/format/json_format.rs src/lib.rs tests/json.rs
git commit -m "$(cat <<'EOF'
Add Json format implementation behind a json Cargo feature

Verified the crate builds and passes its full test suite with only
the json feature enabled (no toml in the dependency tree at all) —
the core claim of this plan. tests/json.rs mirrors
tests/deserialize.rs's default-overlay/bounds coverage via Json
instead of Toml.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Crate docs and final verification

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: an updated crate-level doc example reflecting `PropertyWatcher<T, F>`; no new runtime API.

- [ ] **Step 1: Update the crate-level doc example**

In `src/lib.rs`, change the import line inside the doc comment:

```rust
//! use dyn_properties::{DynProperties, PropertyWatcher};
```

to:

```rust
//! use dyn_properties::{DynProperties, PropertyWatcher, Toml};
```

and change:

```rust
//! let watcher = PropertyWatcher::<AppConfig>::start(
```

to:

```rust
//! let watcher = PropertyWatcher::<AppConfig, Toml>::start(
```

- [ ] **Step 2: Run the doctests**

Run: `cargo test --doc -p dyn-properties --features toml`
Expected: PASS, both doctests compile (the first as `no_run`) and the second executes and passes.

- [ ] **Step 3: Verify the `--features toml` alone build/test/clippy**

Run: `cargo test -p dyn-properties --no-default-features --features toml`
Expected: PASS — full suite (`tests/deserialize.rs`, `tests/watcher.rs`, `tests/smoke.rs`, `tests/validate.rs`, `tests/default.rs`, `tests/custom_format.rs`); `tests/json.rs` is skipped (required-features not satisfied).

Run: `cargo clippy -p dyn-properties --no-default-features --features toml --all-targets -- -D warnings`
Expected: clean, no warnings.

- [ ] **Step 4: Verify the `--features json` alone build/test/clippy**

Run: `cargo test -p dyn-properties --no-default-features --features json`
Expected: PASS — `tests/json.rs`, `tests/smoke.rs`, `tests/validate.rs`, `tests/default.rs`, `tests/custom_format.rs`; `tests/deserialize.rs` and `tests/watcher.rs` are skipped.

Run: `cargo clippy -p dyn-properties --no-default-features --features json --all-targets -- -D warnings`
Expected: clean, no warnings.

- [ ] **Step 5: Verify the `--all-features` build/test/clippy**

Run: `cargo test -p dyn-properties --all-features`
Expected: PASS, every test in the crate.

Run: `cargo clippy -p dyn-properties --all-features --all-targets -- -D warnings`
Expected: clean, no warnings.

Run: `cargo test -p dyn-properties-derive`
Expected: PASS, unaffected by this entire plan (no feature changes there).

- [ ] **Step 6: Verify end-to-end from a fresh external crate using only `json`**

Create `/tmp/dp-json-repro/repro.rs` (outside this repository — do not create this under the `dyn-properties` repo):

```rust
use dyn_properties::{DynProperties, Json, Validate};

#[derive(DynProperties)]
struct Cfg {
    #[range(min = 1, max = 65535)]
    #[default(8080)]
    port: u16,
}

fn main() {
    let cfg: Cfg = Json::parse(br#"{"port": 9000}"#).unwrap();
    assert_eq!(cfg.port, 9000);
    assert!(cfg.validate().is_ok());
    println!("ok: {}", cfg.port);
}
```

Create `/tmp/dp-json-repro/Cargo.toml`:

```toml
[package]
name = "dp-json-repro"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "repro"
path = "repro.rs"

[dependencies]
dyn-properties = { path = "/Users/bgallet/workspace/dyn-properties", default-features = false, features = ["json"] }
```

Run:

```bash
cargo run --manifest-path /tmp/dp-json-repro/Cargo.toml
cargo clippy --manifest-path /tmp/dp-json-repro/Cargo.toml -- -D warnings
```

Expected: both succeed cleanly, and the run prints `ok: 9000`. This proves a real external crate — not this workspace — can depend on `dyn-properties` with only the `json` feature and use it correctly. Then delete the throwaway crate: `rm -rf /tmp/dp-json-repro`.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "$(cat <<'EOF'
Update crate-level doc example for PropertyWatcher<T, F>

Verified: --features toml alone, --features json alone, and
--all-features all build, test, and pass clippy cleanly, plus an
external throwaway crate using only the json feature end-to-end.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
