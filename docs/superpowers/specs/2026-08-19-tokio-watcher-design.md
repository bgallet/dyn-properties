# dyn-properties: Tokio-Native Watcher

## Purpose

`PropertyWatcher` currently always polls its config file from a dedicated
`std::thread`, and change notifications (`subscribe()`) go through a
per-subscriber `std::sync::mpsc` channel. That's the right default for a
dependency-free crate, but a caller who's already running a tokio runtime
would be better served by a watcher that reuses that runtime (via
`tokio::spawn`, no extra OS thread) and uses `tokio::sync::watch` for
notifications — a single-slot, coalescing, memory-bounded mechanism
tokio users already reach for.

This spec adds that as an **opt-in, purely additive** second
implementation, and also reworks the existing threaded watcher's
`subscribe()` for the same "coalescing, bounded" safety property.

## Context: how we got here

Two earlier iterations of this idea were rejected:

1. **Two separate types, `tokio` mandatory-if-enabled**: made
   `PropertyWatcher` itself async-only whenever the `tokio` feature was
   compiled in anywhere in the dependency graph. Rejected — Cargo
   features are supposed to be strictly additive; a feature that changes
   an existing public method's signature can break a consumer who never
   asked for it, if some *other* dependency turns the feature on.
2. **One type, either/or selected by `#[cfg(feature = "tokio")]`**:
   avoids the additivity problem by keeping `PropertyWatcher`'s meaning
   fixed within a given build, but creates a real testing problem —
   `tests/watcher.rs` (threaded-flavored) and a hypothetical
   `tests/tokio_watcher.rs` become mutually exclusive test files, and
   `cargo test --all-features` / the CI `clippy (all)` job would only
   ever exercise *one* of the two backends (whichever `--all-features`
   happens to select), silently starving the other of CI coverage.

This spec's design (both types coexist) avoids both problems: adding the
`tokio` feature only *adds* a new type, and both backends compile and
run together under `--all-features`.

## Goals

- A new `dyn_properties::tokio::PropertyWatcher<T, F>`, gated
  `#[cfg(feature = "tokio")]`, that never spawns an OS thread — its
  background refresh is a `tokio::spawn`'d task on the caller's own
  runtime.
- `dyn_properties::PropertyWatcher<T, F>` (the existing threaded one)
  keeps its name, location, and `start()`/`load()` signatures exactly
  as they are today — zero breaking change.
- Both watchers' `subscribe()` become "coalescing, latest-value-only,
  bounded memory" — no unbounded-growth risk for a subscriber that
  reads slower than the file changes.
- When the threaded watcher's `start()` runs while compiled with the
  `tokio` feature *and* a tokio runtime is actually active in the
  calling context, log a `tracing::warn!` pointing at the tokio-native
  alternative — since that specific combination means a redundant OS
  thread is being spawned when a better option exists right there.

## Non-goals

- No change to `Format`, `Validate`, `Error`, or the derive macro — the
  tokio backend reuses all of it unchanged.
- No unification of the two watcher types behind a shared trait —
  YAGNI for two implementations; revisit only if a third backend shows
  up.
- The runtime-detection warning doesn't fire just because the `tokio`
  feature is compiled in — only when a runtime is genuinely active at
  the call site (see "The warning" below for why).

## The threaded watcher's reworked `subscribe()`

Today's `ChangeSubscription<T>` wraps a fresh `mpsc::channel()` per
subscriber; the background thread fans out to every registered `Sender`
on each change. It queues every change in order but has no bound — a
subscription that's never drained grows forever.

New design: a shared `Condvar`/`Mutex`-guarded generation counter,
alongside the existing `Arc<ArcSwap<T>>`:

```rust
struct Notifier {
    state: Mutex<NotifierState>,
    condvar: Condvar,
}

struct NotifierState {
    generation: u64,
    closed: bool,
}

pub struct ChangeSubscription<T> {
    inner: Arc<ArcSwap<T>>,
    notifier: Arc<Notifier>,
    last_seen_generation: u64,
}
```

`PropertyWatcher` gains a `notifier: Arc<Notifier>` field alongside its
existing `inner`. Each successful, changed reload increments
`generation` and calls `condvar.notify_all()` instead of iterating a
`Vec<Sender>`. `subscribe()` snapshots the current generation and
returns a `ChangeSubscription` pointing at the shared `inner` (for
reading the value) and `notifier` (for waiting).

```rust
impl<T> ChangeSubscription<T> {
    /// Blocks until the value has changed since the last call (or since
    /// `subscribe()`), then returns the *latest* value. If multiple
    /// changes happen between two calls, only the latest is observed —
    /// intermediate values are not queued.
    pub fn wait_for_change(&mut self) -> Option<Arc<T>> { ... }

    /// Like `wait_for_change`, but gives up after `timeout`.
    pub fn wait_for_change_timeout(&mut self, timeout: Duration) -> Option<Arc<T>> { ... }
}

impl<T> Iterator for ChangeSubscription<T> {
    type Item = Arc<T>;
    fn next(&mut self) -> Option<Arc<T>> { self.wait_for_change() }
}
```

This replaces `recv()`/`recv_timeout()` (renamed to reflect the new
"give me the next change" rather than "channel receive" semantics — a
pre-release rename, `subscribe()` shipped in the immediately preceding
PR and has no external consumers yet).

Watcher shutdown (the existing `stop_tx` disconnect path) additionally
locks `notifier.state`, sets `closed = true`, and calls `notify_all()`
so blocked waiters wake and observe closure (returning `None`) instead
of hanging — mirroring how the mpsc-based version let subscribers
detect "sender dropped" for free; `Condvar` needs this done explicitly.

## The tokio-native watcher

```rust
// src/tokio_watcher.rs, re-exported as dyn_properties::tokio::PropertyWatcher
pub struct PropertyWatcher<T, F> {
    watch_tx: tokio::sync::watch::Sender<Arc<T>>,
    handle: tokio::task::JoinHandle<()>,
    _format: PhantomData<F>,
}

impl<T, F> PropertyWatcher<T, F>
where
    T: serde::de::DeserializeOwned + Validate + Default + Send + Sync + 'static,
    F: Format + Send + Sync + 'static,
{
    pub async fn start(path: impl Into<PathBuf>, interval: Duration) -> Result<Self, Error> {
        let path = path.into();
        let (initial_bytes, initial) = load_and_validate::<T, F>(&path).await?;
        let (watch_tx, _rx) = tokio::sync::watch::channel(Arc::new(initial));

        let watch_path = path.clone();
        let tx = watch_tx.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // first tick fires immediately; initial load already happened
            let mut last_bytes = initial_bytes;
            loop {
                ticker.tick().await;
                let bytes = match tokio::fs::read(&watch_path).await {
                    Ok(bytes) => bytes,
                    Err(e) => { tracing::warn!(...); continue; }
                };
                if bytes == last_bytes {
                    continue;
                }
                match parse_and_validate::<T, F>(&bytes) {
                    Ok(value) => { let _ = tx.send(Arc::new(value)); }
                    Err(e) => { tracing::warn!(...); }
                }
                last_bytes = bytes;
            }
        });

        Ok(PropertyWatcher { watch_tx, handle, _format: PhantomData })
    }

    /// Returns the current value. Unlike the threaded watcher's `load()`
    /// (an `arc_swap::Guard`), this is a `tokio::sync::watch::Ref` —
    /// same "use and drop quickly" guidance applies.
    pub fn load(&self) -> tokio::sync::watch::Ref<'_, Arc<T>> {
        self.watch_tx.borrow()
    }

    /// Returns tokio's own `watch::Receiver` directly — no wrapper type.
    /// `.changed().await` waits for the next update; `.borrow()` peeks
    /// the current value without waiting.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Arc<T>> {
        self.watch_tx.subscribe()
    }
}

impl<T, F> Drop for PropertyWatcher<T, F> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
```

Notes:

- No `arc-swap` involved in this backend at all — `tokio::sync::watch`
  is the single source of truth for both reading the current value and
  waiting for changes, so there's nothing left for `ArcSwap` to do.
- Same "no panic-catching" and "byte-comparison gate before
  parse/validate" behavior as the threaded watcher, for consistency
  (both were deliberate simplifications made to the threaded watcher
  recently; this backend should match, not regress to the old
  panic-catching design).
- `parse_and_validate<T, F>(bytes: &[u8]) -> Result<T, Error>` is
  reused unchanged from `src/watcher.rs` (marked `pub(crate)` instead
  of private). `load_and_validate` is *not* shared — the threaded
  watcher's version uses `std::fs::read`, this one uses
  `tokio::fs::read(...).await`; each stays local to its own module.
- Referencing the external `tokio` crate from inside a module literally
  named `tokio` (`dyn_properties::tokio`) needs care to avoid
  ambiguity — use `::tokio::...` (leading `::`, absolute path) for
  every reference to the dependency inside `src/tokio_watcher.rs`.

## The warning

```rust
// inside the threaded PropertyWatcher::start(), gated:
#[cfg(feature = "tokio")]
if tokio::runtime::Handle::try_current().is_ok() {
    tracing::warn!(
        "dyn-properties: starting a thread-based PropertyWatcher while a tokio runtime \
         is active — consider dyn_properties::tokio::PropertyWatcher instead to avoid \
         spawning a dedicated OS thread"
    );
}
```

Checking `Handle::try_current()` (not just "is the `tokio` feature
compiled in") matters: the feature can be on in a build for reasons
unrelated to the calling code path (another part of the dependency
graph, or a binary that uses both a sync entry point and an unrelated
async subsystem elsewhere) — warning unconditionally in that case would
recommend an alternative that doesn't even apply to the caller's
context. Checking for an active runtime at the actual call site targets
the warning at the one situation where it's unambiguously true: a
redundant OS thread is being spawned right now, when `tokio::spawn`
was available instead.

This fires on every `start()` call made under those conditions (not
once-per-process) — each call really does spawn a new, individually
avoidable thread.

## Cargo changes

```toml
[dependencies]
arc-swap = "1"  # threaded watcher only; stays unconditional (small, harmless if unused)
tokio = { version = "1", features = ["rt", "time", "fs"], optional = true }

[dev-dependencies]
tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros", "time", "fs"] }

[features]
tokio = ["dep:tokio"]

[[test]]
name = "tokio_watcher"
required-features = ["toml", "tokio"]
```

> **Erratum:** the `tokio` dependency's feature list above also needed
> `sync` (for `tokio::sync::watch`), which this spec omitted — caught
> during implementation.

`tokio` as a dev-dependency is unconditional (dev-dependencies don't
affect what a consumer of the published crate pulls in) — needed so
`tests/tokio_watcher.rs` can use `#[tokio::test]` regardless of the
library's own `tokio` feature state; the test *file* is still gated via
`required-features` so it only actually runs (and only needs
`dyn_properties::tokio::PropertyWatcher` to exist) when `--features
tokio` is passed.

## Module layout

- `src/watcher.rs` — threaded watcher, unconditional (as today), plus
  the `ChangeSubscription` rework and the new warning.
  `parse_and_validate` becomes `pub(crate)`.
- `src/tokio_watcher.rs` — new, `#[cfg(feature = "tokio")]`.
- `src/lib.rs`:
  ```rust
  pub use watcher::{ChangeSubscription, PropertyWatcher};

  #[cfg(feature = "tokio")]
  pub mod tokio {
      pub use crate::tokio_watcher::PropertyWatcher;
  }
  ```

## Testing strategy

- `tests/watcher.rs` (existing): update to the renamed
  `wait_for_change`/`wait_for_change_timeout` API; existing subscriber
  tests (`subscriber_receives_new_value_on_change`,
  `multiple_subscribers_all_receive_the_same_change`,
  `dropping_the_watcher_ends_the_subscription`) keep their intent, just
  called through the new methods. Add two new `#[cfg(feature =
  "tokio")]`-gated tests in this same file: the warning fires when
  `start()` runs inside a `#[tokio::test]` context, and does *not* fire
  when it runs in a plain `#[test]` (no runtime active) — both compiled
  in only when `tokio` is enabled, so the file's `required-features`
  stays `["toml"]` unchanged.
- New `tests/tokio_watcher.rs`, `required-features = ["toml", "tokio"]`,
  mirroring `tests/watcher.rs`'s coverage for the tokio backend: initial
  load, reload picks up changes, invalid reload keeps last-good value
  and logs, `subscribe()` delivers the latest value via
  `.changed().await`/`.borrow()`, dropping the watcher makes
  `.changed().await` return `Err`.
- Clippy feature matrix (`.github/workflows/ci.yml`) gains a `tokio`
  entry (`--no-default-features --features tokio`, proving the tokio
  backend compiles standalone without `toml`/`json`) alongside the
  existing `none`/`toml`/`json`/`all` entries; branch protection's
  required status checks list gains `clippy (tokio)`.
- `cargo test --workspace --all-features` continues to be the "run
  everything" command — now genuinely runs both backends together,
  which is the whole point of the additive design.

## Documentation

`src/lib.rs`'s crate-level doc comment gains a short "## Tokio" section
(alongside the existing "## Required fields"/"## Cargo features"
sections) pointing at `dyn_properties::tokio::PropertyWatcher`, its
`#[cfg(feature = "tokio")]` gate, and why it exists (no OS thread,
`tokio::sync::watch` notifications). `README.md` gets the equivalent
one- or two-sentence mention under its own Cargo features list.

## Compatibility

Purely additive: existing code using `dyn_properties::PropertyWatcher`
is unaffected except for the `subscribe()` rename (pre-release, no
external consumers). No existing dependency versions or features
change; `tokio` defaults to off, matching `toml`/`json`.
