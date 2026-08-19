# Tokio-Native Watcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in, purely additive `dyn_properties::tokio::PropertyWatcher` that
spawns no OS thread (background refresh via `tokio::spawn`/`tokio::time::interval`,
notifications via `tokio::sync::watch`), rework the existing threaded
`PropertyWatcher`'s `subscribe()` to the same coalescing/bounded-memory design, and warn
when the threaded watcher is started under an active tokio runtime while compiled with
the `tokio` feature.

**Architecture:** Two independent `PropertyWatcher` implementations live side by side:
the existing `dyn_properties::PropertyWatcher` (unconditional, `std::thread`-based) and
a new `dyn_properties::tokio::PropertyWatcher` (gated `#[cfg(feature = "tokio")]`,
`tokio::spawn`-based). Both call the same shared `dyn_properties::watcher::parse_and_validate`
(made `pub(crate)`) for the bytes→`T` step; each backend owns its own I/O
(`std::fs::read` vs `tokio::fs::read`) and its own change-notification mechanism
(`Condvar`-based `ChangeSubscription<T>` vs native `tokio::sync::watch::Receiver<Arc<T>>`).

**Tech Stack:** Rust (edition 2024), `tokio` (new optional dependency: `rt`, `time`,
`fs` features), `arc-swap` (threaded watcher only, unchanged), `tracing`, `tracing-test`
(dev-only).

**Spec:** `docs/superpowers/specs/2026-08-19-tokio-watcher-design.md`

## Global Constraints

- No change to `Format`, `Validate`, `Error`, or `dyn-properties-derive` — both
  backends reuse all of it unchanged.
- `dyn_properties::PropertyWatcher`'s name, location, and `start()`/`load()` signatures
  do not change. The only breaking change anywhere in this plan is the pre-release
  rename of `ChangeSubscription::recv`/`recv_timeout` to `wait_for_change`/
  `wait_for_change_timeout` (shipped in the immediately preceding PR, no external
  consumers yet — acceptable).
- `tokio` Cargo feature defaults **off**, matching `toml`/`json`.
- Both backends: no panic-catching around a reload tick (a panic ends the background
  thread/task, `load()` keeps serving the last good value) and the byte-comparison gate
  before parse/validate (skip parse/validate/notify entirely when the file's raw bytes
  are unchanged since the last read) — these are deliberate properties of the existing
  threaded watcher; the tokio backend must match them, not reintroduce the
  panic-catching design from before it was simplified.
- After every task: `cargo build --workspace --all-features --tests` clean; `cargo test
  --workspace --all-features` all green; `cargo fmt --all -- --check` clean; `cargo
  clippy --workspace --all-targets -- -D warnings` clean for each of
  `--no-default-features`, `--no-default-features --features toml`,
  `--no-default-features --features json`, `--no-default-features --features tokio`,
  `--all-features`.

---

### Task 1: Add `tokio` as an optional Cargo dependency and feature

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: the `tokio` optional dependency (library) and unconditional dev-dependency
  (tests), the `tokio` Cargo feature, and a `tokio_watcher` `[[test]]` entry — all
  consumed by Tasks 2–4.

- [ ] **Step 1: Add the optional `tokio` dependency and feature**

In `Cargo.toml`, in the `[dependencies]` table, add (after `arc-swap = "1"`, before
`tracing = "0.1"`):

```toml
tokio = { version = "1", features = ["rt", "time", "fs"], optional = true }
```

In `[dev-dependencies]`, add:

```toml
tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros", "time", "fs"] }
```

In `[features]`, add a new line:

```toml
tokio = ["dep:tokio"]
```

(Final `[features]` table should read `toml = ["dep:toml"]`, `json = ["dep:serde_json"]`,
`tokio = ["dep:tokio"]`, in that order.)

- [ ] **Step 2: Add the `tokio_watcher` test target**

Add a new `[[test]]` block, after the existing `[[test]] name = "json"` block and before
`[workspace]`:

```toml
[[test]]
name = "tokio_watcher"
required-features = ["toml", "tokio"]
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p dyn-properties --features tokio`
Expected: succeeds (nothing references the `tokio` crate yet, but the dependency
resolves and the feature compiles).

Run: `cargo build -p dyn-properties` (no features)
Expected: succeeds unchanged — `tokio` is not pulled in.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "Add tokio as an optional dependency and Cargo feature"
```

---

### Task 2: Rework `ChangeSubscription` to a coalescing design; add the tokio-runtime warning

**Files:**
- Modify: `src/watcher.rs`
- Modify: `tests/watcher.rs`

**Interfaces:**
- Consumes: the `tokio` feature from Task 1 (for the `#[cfg(feature = "tokio")]`-gated
  warning code and its tests).
- Produces: `pub(crate) fn parse_and_validate<T, F>(bytes: &[u8]) -> Result<T, Error>` in
  `src/watcher.rs` (was private `fn`) — consumed by Task 3's `src/tokio_watcher.rs`.
  `ChangeSubscription<T>::wait_for_change(&mut self) -> Option<Arc<T>>` and
  `wait_for_change_timeout(&mut self, timeout: Duration) -> Option<Arc<T>>`, replacing
  `recv`/`recv_timeout`.

- [ ] **Step 1: Replace `src/watcher.rs` in full**

Replace the entire contents of `src/watcher.rs` with:

```rust
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, Guard};

use crate::{Error, Format, Validate};

/// A config file loaded once and then polled on a background thread, exposing the latest
/// successfully-parsed-and-validated value via [`load`](Self::load), and optionally
/// notifying [subscribers](Self::subscribe) whenever the file's content actually changes.
///
/// `F` selects the file format (e.g. [`Toml`](crate::Toml), [`Json`](crate::Json), or a
/// caller-defined [`Format`] implementation) — see [`Format`] for why this is a type
/// parameter rather than runtime extension-sniffing.
///
/// A failed reload (I/O error, parse error, or a `Validate` bound violation) is logged
/// via `tracing` and discarded, leaving the previously-loaded value in place. A *panicking*
/// reload (e.g. a bug in a caller-defined `Format::parse`) is not caught: it ends the
/// background thread, permanently stopping further reloads, though [`load`](Self::load)
/// keeps serving the last successfully-loaded value indefinitely. Dropping the watcher
/// signals the background thread to stop polling.
///
/// Config files should be updated atomically (write to a temp file in the same
/// directory, then rename over the target) rather than truncated in place — a poll
/// tick that reads a mid-write, momentarily-empty file will parse and validate
/// successfully as "everything defaulted" under this crate's default-overlay design,
/// silently replacing good config rather than failing loudly.
///
/// If a tokio runtime is active when [`start`](Self::start) is called and this crate is
/// compiled with the `tokio` feature, a `tracing::warn!` points at
/// [`dyn_properties::tokio::PropertyWatcher`](crate::tokio::PropertyWatcher) instead,
/// which spawns no OS thread.
pub struct PropertyWatcher<T, F> {
    inner: Arc<ArcSwap<T>>,
    notifier: Arc<Notifier>,
    /// Never read after construction — dropping it disconnects the channel, which is what
    /// signals the background thread's `recv_timeout` to wake immediately and stop
    /// polling. The leading underscore tells the `dead_code` lint this is intentional.
    _stop: mpsc::Sender<()>,
    _format: PhantomData<F>,
}

impl<T, F> PropertyWatcher<T, F>
where
    T: serde::de::DeserializeOwned + Validate + Default + Send + Sync + 'static,
    F: Format + Send + Sync + 'static,
{
    /// Loads and validates `path` once, then spawns a background thread that re-loads it
    /// every `interval`, replacing the stored value on success and keeping the previous
    /// one (with a logged warning) on failure.
    ///
    /// Returns an error if the initial load fails; the background thread is only spawned
    /// once the initial load succeeds.
    pub fn start(path: impl Into<PathBuf>, interval: Duration) -> Result<Self, Error> {
        let path = path.into();
        let (initial_bytes, initial) = load_and_validate::<T, F>(&path)?;
        let inner = Arc::new(ArcSwap::new(Arc::new(initial)));
        let notifier = Arc::new(Notifier::new());

        let watcher_inner = Arc::clone(&inner);
        let watcher_notifier = Arc::clone(&notifier);
        let watch_path = path.clone();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        // Carries the caller's current tracing span onto the background thread, so
        // reload logs stay correlated with whatever context `start` was called from
        // (a fresh OS thread otherwise starts with no span of its own).
        let span = tracing::Span::current();
        thread::spawn(move || {
            let _entered = span.enter();
            let mut last_bytes = initial_bytes;
            loop {
                // Doubles as the poll interval's sleep and the stop signal: a normal
                // timeout means "reload now", while a disconnect (the watcher was
                // dropped) wakes this up immediately instead of waiting out the rest
                // of the interval.
                match stop_rx.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        watcher_notifier.close();
                        return;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                let bytes = match std::fs::read(&watch_path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::warn!(
                            path = %watch_path.display(),
                            error = %Error::from(e),
                            "dyn-properties: reload failed, keeping previous value"
                        );
                        continue;
                    }
                };
                if bytes == last_bytes {
                    continue;
                }
                // The file's content did change: parse+validate it, and either way (success
                // or failure) treat this exact content as "already handled" so a
                // persistently-invalid-but-unchanging file doesn't re-log every tick.
                match parse_and_validate::<T, F>(&bytes) {
                    Ok(value) => {
                        watcher_inner.store(Arc::new(value));
                        watcher_notifier.notify_change();
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %watch_path.display(),
                            error = %e,
                            "dyn-properties: reload failed, keeping previous value"
                        );
                    }
                }
                last_bytes = bytes;
            }
        });

        #[cfg(feature = "tokio")]
        if ::tokio::runtime::Handle::try_current().is_ok() {
            tracing::warn!(
                "dyn-properties: starting a thread-based PropertyWatcher while a tokio \
                 runtime is active — consider dyn_properties::tokio::PropertyWatcher \
                 instead to avoid spawning a dedicated OS thread"
            );
        }

        Ok(PropertyWatcher {
            inner,
            notifier,
            _stop: stop_tx,
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

    /// Subscribes to future changes: [`ChangeSubscription::wait_for_change`] blocks
    /// until the file's raw content is genuinely different from what it was at
    /// `subscribe()` time (or at the last `wait_for_change` call) and the new content
    /// parses and validates successfully, then returns the *latest* value.
    ///
    /// Only *future* changes are delivered — subscribing doesn't replay the current
    /// value; call [`load`](Self::load) for that. Like `tokio::sync::watch`, this
    /// coalesces: if several changes happen between two `wait_for_change` calls, only
    /// the latest is observed, and memory use never grows regardless of how many
    /// changes happen or how slowly a subscriber reads.
    pub fn subscribe(&self) -> ChangeSubscription<T> {
        ChangeSubscription {
            inner: Arc::clone(&self.inner),
            notifier: Arc::clone(&self.notifier),
            last_seen_generation: self.notifier.current_generation(),
        }
    }
}

struct Notifier {
    state: Mutex<NotifierState>,
    condvar: Condvar,
}

struct NotifierState {
    generation: u64,
    closed: bool,
}

impl Notifier {
    fn new() -> Self {
        Notifier {
            state: Mutex::new(NotifierState {
                generation: 0,
                closed: false,
            }),
            condvar: Condvar::new(),
        }
    }

    fn current_generation(&self) -> u64 {
        self.state.lock().unwrap().generation
    }

    fn notify_change(&self) {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        self.condvar.notify_all();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        self.condvar.notify_all();
    }
}

/// A subscription to a [`PropertyWatcher`]'s future changes, obtained from
/// [`PropertyWatcher::subscribe`].
pub struct ChangeSubscription<T> {
    inner: Arc<ArcSwap<T>>,
    notifier: Arc<Notifier>,
    last_seen_generation: u64,
}

impl<T> ChangeSubscription<T> {
    /// Blocks until the value has changed since the last call (or since
    /// `subscribe()`), then returns the *latest* value, or `None` once the watcher has
    /// been dropped and no further changes can ever arrive. If multiple changes happen
    /// between two calls, only the latest is observed — intermediate values are not
    /// queued.
    pub fn wait_for_change(&mut self) -> Option<Arc<T>> {
        let mut state = self.notifier.state.lock().unwrap();
        loop {
            if state.generation != self.last_seen_generation {
                self.last_seen_generation = state.generation;
                return Some(self.inner.load_full());
            }
            if state.closed {
                return None;
            }
            state = self.notifier.condvar.wait(state).unwrap();
        }
    }

    /// Like [`wait_for_change`](Self::wait_for_change), but gives up and returns `None`
    /// after `timeout` if no change arrives (this collapses "timed out" and "watcher
    /// dropped" into the same `None`, same as `wait_for_change`'s "no more changes are
    /// coming" result).
    pub fn wait_for_change_timeout(&mut self, timeout: Duration) -> Option<Arc<T>> {
        let mut state = self.notifier.state.lock().unwrap();
        let deadline = Instant::now() + timeout;
        loop {
            if state.generation != self.last_seen_generation {
                self.last_seen_generation = state.generation;
                return Some(self.inner.load_full());
            }
            if state.closed {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .notifier
                .condvar
                .wait_timeout(state, deadline - now)
                .unwrap();
            state = guard;
            // Loop back around regardless of whether this was a real notification, a
            // timeout, or a spurious wakeup: the generation/closed checks above and the
            // deadline check on the next iteration decide what actually happened.
        }
    }
}

impl<T> Iterator for ChangeSubscription<T> {
    type Item = Arc<T>;

    fn next(&mut self) -> Option<Arc<T>> {
        self.wait_for_change()
    }
}

pub(crate) fn parse_and_validate<T, F>(bytes: &[u8]) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned + Validate,
    F: Format,
{
    let value: T = F::parse(bytes).map_err(|e| Error::Parse(Box::new(e)))?;
    value.validate()?;
    Ok(value)
}

fn load_and_validate<T, F>(path: &Path) -> Result<(Vec<u8>, T), Error>
where
    T: serde::de::DeserializeOwned + Validate,
    F: Format,
{
    let bytes = std::fs::read(path)?;
    let value = parse_and_validate::<T, F>(&bytes)?;
    Ok((bytes, value))
}
```

- [ ] **Step 2: Update `tests/watcher.rs`'s existing subscriber tests**

In `tests/watcher.rs`, apply these changes:

1. In `subscriber_receives_new_value_on_change`: change `let subscription =
   watcher.subscribe();` to `let mut subscription = watcher.subscribe();`, and
   `.recv_timeout(` to `.wait_for_change_timeout(`.

2. In `subscriber_gets_no_notification_for_a_byte_identical_rewrite`: change `let
   subscription = watcher.subscribe();` to `let mut subscription =
   watcher.subscribe();`, and `.recv_timeout(` to `.wait_for_change_timeout(`.

3. In `subscriber_gets_no_notification_before_subscribing`: change `let subscription =
   watcher.subscribe();` to `let mut subscription = watcher.subscribe();`, and
   `.recv_timeout(` to `.wait_for_change_timeout(`.

4. In `multiple_subscribers_all_receive_the_same_change`: change `let sub_a =
   watcher.subscribe();` to `let mut sub_a = watcher.subscribe();`, `let sub_b =
   watcher.subscribe();` to `let mut sub_b = watcher.subscribe();`, and both
   `.recv_timeout(` to `.wait_for_change_timeout(`.

5. In `dropping_the_watcher_ends_the_subscription`: change `let subscription =
   watcher.subscribe();` to `let mut subscription = watcher.subscribe();`, and
   `.recv_timeout(` to `.wait_for_change_timeout(`.

- [ ] **Step 3: Add a coalescing test**

Add to `tests/watcher.rs`, after `multiple_subscribers_all_receive_the_same_change`:

```rust
#[test]
fn subscriber_only_sees_the_latest_value_after_multiple_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();
    let mut subscription = watcher.subscribe();

    std::fs::write(file.path(), "port = 9100").unwrap();
    std::thread::sleep(Duration::from_millis(80));
    std::fs::write(file.path(), "port = 9200").unwrap();
    std::thread::sleep(Duration::from_millis(80));
    std::fs::write(file.path(), "port = 9300").unwrap();

    // A single wait_for_change call coalesces all three intervening changes into the
    // latest value — no backlog of 9100/9200 to drain first.
    let received = subscription
        .wait_for_change_timeout(Duration::from_secs(5))
        .expect("expected a change notification");
    assert_eq!(received.port, 9300);
}
```

- [ ] **Step 4: Add the runtime-detection warning tests**

Add to `tests/watcher.rs`, at the end of the file:

```rust
#[tokio::test]
#[cfg(feature = "tokio")]
#[traced_test]
async fn starting_threaded_watcher_inside_a_tokio_runtime_logs_a_warning() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let _watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).unwrap();

    assert!(logs_contain("consider dyn_properties::tokio::PropertyWatcher"));
}

#[test]
#[cfg(feature = "tokio")]
#[traced_test]
fn starting_threaded_watcher_outside_a_tokio_runtime_does_not_warn() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let _watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).unwrap();

    assert!(!logs_contain("consider dyn_properties::tokio::PropertyWatcher"));
}
```

- [ ] **Step 5: Run tests without the `tokio` feature**

Run: `cargo test --features toml --test watcher`
Expected: all tests pass (the two new warning tests are absent from this build, since
`tokio` isn't enabled).

- [ ] **Step 6: Run tests with the `tokio` feature**

Run: `cargo test --features "toml tokio" --test watcher`
Expected: all tests pass, including the two new warning tests.

- [ ] **Step 7: Run the full verification matrix**

Run: `cargo build --workspace --all-features --tests`
Run: `cargo test --workspace --all-features`
Run: `cargo fmt --all -- --check` (run `cargo fmt --all` first if it reports diffs)
Run: `cargo clippy --workspace --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features toml --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features json --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features tokio --all-targets -- -D warnings`
Run: `cargo clippy --workspace --all-features --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 8: Commit**

```bash
git add src/watcher.rs tests/watcher.rs
git commit -m "Rework ChangeSubscription to a coalescing design; warn when threaded watcher starts under an active tokio runtime"
```

---

### Task 3: Add `dyn_properties::tokio::PropertyWatcher`

**Files:**
- Create: `src/tokio_watcher.rs`
- Modify: `src/lib.rs`
- Create: `tests/tokio_watcher.rs`

**Interfaces:**
- Consumes: `crate::watcher::parse_and_validate` (Task 2, now `pub(crate)`);
  `crate::{Error, Format, Validate}` (unchanged).
- Produces: `dyn_properties::tokio::PropertyWatcher<T, F>` with `async fn
  start(path, interval) -> Result<Self, Error>`, `fn load(&self) ->
  tokio::sync::watch::Ref<'_, Arc<T>>`, `fn subscribe(&self) ->
  tokio::sync::watch::Receiver<Arc<T>>`.

- [ ] **Step 1: Create `src/tokio_watcher.rs`**

```rust
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::watcher::parse_and_validate;
use crate::{Error, Format, Validate};

/// A tokio-native counterpart to [`crate::PropertyWatcher`]: reads a config file once,
/// then re-loads it on a `tokio::spawn`'d task instead of a dedicated OS thread, and
/// delivers change notifications through a single `tokio::sync::watch` channel that
/// backs both [`load`](Self::load) and [`subscribe`](Self::subscribe).
///
/// Behaves identically to [`crate::PropertyWatcher`] otherwise: a failed reload (I/O
/// error, parse error, or a `Validate` bound violation) is logged via `tracing` and
/// discarded, leaving the previously-loaded value in place; a *panicking* reload ends
/// the background task, permanently stopping further reloads, though [`load`](Self::load)
/// keeps serving the last successfully-loaded value indefinitely. Dropping the watcher
/// aborts the background task.
///
/// Config files should be updated atomically (write to a temp file in the same
/// directory, then rename over the target) — see [`crate::PropertyWatcher`]'s docs for
/// why.
pub struct PropertyWatcher<T, F> {
    watch_tx: ::tokio::sync::watch::Sender<Arc<T>>,
    handle: ::tokio::task::JoinHandle<()>,
    _format: PhantomData<F>,
}

impl<T, F> PropertyWatcher<T, F>
where
    T: serde::de::DeserializeOwned + Validate + Default + Send + Sync + 'static,
    F: Format + Send + Sync + 'static,
{
    /// Loads and validates `path` once, then spawns a `tokio::spawn`'d task that
    /// re-loads it every `interval`, replacing the stored value on success and keeping
    /// the previous one (with a logged warning) on failure.
    ///
    /// Returns an error if the initial load fails; the background task is only spawned
    /// once the initial load succeeds. Must be called from within a tokio runtime — the
    /// same requirement `tokio::spawn` itself has.
    pub async fn start(path: impl Into<PathBuf>, interval: Duration) -> Result<Self, Error> {
        let path = path.into();
        let (initial_bytes, initial) = load_and_validate::<T, F>(&path).await?;
        let (watch_tx, _watch_rx) = ::tokio::sync::watch::channel(Arc::new(initial));

        let watch_path = path.clone();
        let tx = watch_tx.clone();
        let handle = ::tokio::spawn(async move {
            let mut ticker = ::tokio::time::interval(interval);
            ticker.tick().await; // first tick fires immediately; the initial load above already happened
            let mut last_bytes = initial_bytes;
            loop {
                ticker.tick().await;
                let bytes = match ::tokio::fs::read(&watch_path).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::warn!(
                            path = %watch_path.display(),
                            error = %Error::from(e),
                            "dyn-properties: reload failed, keeping previous value"
                        );
                        continue;
                    }
                };
                if bytes == last_bytes {
                    continue;
                }
                match parse_and_validate::<T, F>(&bytes) {
                    Ok(value) => {
                        let _ = tx.send(Arc::new(value));
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %watch_path.display(),
                            error = %e,
                            "dyn-properties: reload failed, keeping previous value"
                        );
                    }
                }
                last_bytes = bytes;
            }
        });

        Ok(PropertyWatcher {
            watch_tx,
            handle,
            _format: PhantomData,
        })
    }

    /// Returns the current value.
    ///
    /// Unlike [`crate::PropertyWatcher::load`] (which returns an `arc_swap::Guard`),
    /// this returns a `tokio::sync::watch::Ref`: the same "use and drop quickly, don't
    /// hold across an `.await`" guidance applies. Clone the `Arc` out first
    /// (`Arc::clone(&*watcher.load())`) to carry the value across an `.await` point or
    /// into another task.
    pub fn load(&self) -> ::tokio::sync::watch::Ref<'_, Arc<T>> {
        self.watch_tx.borrow()
    }

    /// Subscribes to future changes: returns tokio's own `watch::Receiver` directly, so
    /// the full `tokio::sync::watch` API is available — `.changed().await` waits for
    /// the next update, `.borrow()`/`.borrow_and_update()` peek the current value.
    /// Coalescing: if several changes happen between two `.changed().await` calls, only
    /// the latest value is observed.
    pub fn subscribe(&self) -> ::tokio::sync::watch::Receiver<Arc<T>> {
        self.watch_tx.subscribe()
    }
}

impl<T, F> Drop for PropertyWatcher<T, F> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn load_and_validate<T, F>(path: &Path) -> Result<(Vec<u8>, T), Error>
where
    T: serde::de::DeserializeOwned + Validate,
    F: Format,
{
    let bytes = ::tokio::fs::read(path).await?;
    let value = parse_and_validate::<T, F>(&bytes)?;
    Ok((bytes, value))
}
```

- [ ] **Step 2: Wire it into `src/lib.rs`**

In `src/lib.rs`, after the existing `mod watcher;` / `pub use watcher::{ChangeSubscription,
PropertyWatcher};` lines, add:

```rust
#[cfg(feature = "tokio")]
mod tokio_watcher;

#[cfg(feature = "tokio")]
pub mod tokio {
    //! A tokio-native [`PropertyWatcher`](crate::PropertyWatcher) that spawns no OS
    //! thread — background refresh runs as a `tokio::spawn`'d task, and change
    //! notifications are delivered via `tokio::sync::watch`.
    pub use crate::tokio_watcher::PropertyWatcher;
}
```

- [ ] **Step 3: Create `tests/tokio_watcher.rs`**

```rust
use dyn_properties::tokio::PropertyWatcher;
use dyn_properties::{DynProperties, Toml};
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

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(watcher.load().port, 9000);
}

#[tokio::test]
async fn start_fails_on_invalid_initial_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 0").unwrap();

    let result = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).await;
    match result {
        Err(dyn_properties::Error::Validation { .. }) => {}
        Err(other) => panic!("expected Error::Validation, got {other:?}"),
        Ok(_) => panic!("expected start() to fail on out-of-range port"),
    }
}

#[tokio::test]
async fn start_fails_with_error_parse_on_unparseable_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "not valid = [").unwrap();

    let result = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).await;
    match result {
        Err(dyn_properties::Error::Parse(_)) => {}
        Err(other) => panic!("expected Error::Parse, got {other:?}"),
        Ok(_) => panic!("expected start() to fail on unparseable TOML"),
    }
}

#[tokio::test]
async fn reload_picks_up_valid_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
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

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();

    std::fs::write(file.path(), "port = 0").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload failed"));
    assert!(logs_contain("port: 0 is out of range"));
}

#[tokio::test]
async fn subscribe_delivers_the_latest_value() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    std::fs::write(file.path(), "port = 9500").unwrap();
    rx.changed().await.unwrap();

    assert_eq!(rx.borrow().port, 9500);
}

#[tokio::test]
async fn subscribe_coalesces_multiple_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    std::fs::write(file.path(), "port = 9100").unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    std::fs::write(file.path(), "port = 9200").unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    std::fs::write(file.path(), "port = 9300").unwrap();

    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().port, 9300);
}

#[tokio::test]
async fn dropping_the_watcher_ends_the_subscription() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    drop(watcher);

    assert!(rx.changed().await.is_err());
}
```

- [ ] **Step 4: Run the new tests**

Run: `cargo test --features "toml tokio" --test tokio_watcher`
Expected: all 9 tests pass.

- [ ] **Step 5: Run the full verification matrix**

Run: `cargo build --workspace --all-features --tests`
Run: `cargo test --workspace --all-features`
Run: `cargo fmt --all -- --check` (run `cargo fmt --all` first if it reports diffs)
Run: `cargo clippy --workspace --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features toml --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features json --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features tokio --all-targets -- -D warnings`
Run: `cargo clippy --workspace --all-features --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 6: Commit**

```bash
git add src/tokio_watcher.rs src/lib.rs tests/tokio_watcher.rs
git commit -m "Add dyn_properties::tokio::PropertyWatcher"
```

---

### Task 4: CI matrix and docs

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `src/lib.rs`
- Modify: `README.md`

**Interfaces:**
- Consumes: the `tokio` feature (Task 1) and `dyn_properties::tokio::PropertyWatcher`
  (Task 3) — this task only documents/tests what already exists, no new library code.

- [ ] **Step 1: Add a `tokio` entry to the clippy matrix**

In `.github/workflows/ci.yml`, change:

```yaml
        features: [none, toml, json, all]
```

to:

```yaml
        features: [none, toml, json, tokio, all]
```

- [ ] **Step 2: Add a "## Tokio" section to `src/lib.rs`'s crate doc comment**

In `src/lib.rs`, replace the existing `## Cargo features` section:

```rust
//! ## Cargo features
//!
//! Neither format is enabled by default — enable exactly the one(s) you need:
//!
//! - `toml` — adds [`Toml`], parsing config files as TOML.
//! - `json` — adds [`Json`], parsing config files as JSON.
//!
//! Both can be enabled together. With neither enabled, [`Format`] itself is still
//! available — implement it for your own format (YAML, RON, ...) and use
//! `PropertyWatcher<T, YourFormat>` without depending on `toml` or `serde_json` at all.
```

with:

```rust
//! ## Cargo features
//!
//! Neither format is enabled by default — enable exactly the one(s) you need:
//!
//! - `toml` — adds [`Toml`], parsing config files as TOML.
//! - `json` — adds [`Json`], parsing config files as JSON.
//! - `tokio` — adds [`tokio::PropertyWatcher`], a tokio-native counterpart to the
//!   default thread-based [`PropertyWatcher`] (see "Tokio" below).
//!
//! The two format features can be enabled together. With neither enabled, [`Format`]
//! itself is still available — implement it for your own format (YAML, RON, ...) and
//! use `PropertyWatcher<T, YourFormat>` without depending on `toml` or `serde_json` at
//! all.
//!
//! ## Tokio
//!
//! [`PropertyWatcher`] always works: it polls its file from a dedicated `std::thread`,
//! no async runtime required. If you're already running a tokio runtime, enable the
//! `tokio` feature and use [`tokio::PropertyWatcher`] instead — it spawns no extra OS
//! thread (refresh runs as a `tokio::spawn`'d task on your own runtime) and its
//! `subscribe()` returns a native `tokio::sync::watch::Receiver`. If you start the
//! thread-based [`PropertyWatcher`] while a tokio runtime is active and the `tokio`
//! feature is enabled, a `tracing::warn!` points you at the alternative.
```

- [ ] **Step 3: Mention `tokio` in `README.md`**

In `README.md`, find the `## Cargo features` section and its bullet list (`- `toml` —
...` / `- `json` — ...`). Add a third bullet:

```markdown
- `tokio` — adds a tokio-native `PropertyWatcher` under `dyn_properties::tokio` that
  spawns no OS thread (background refresh runs as a `tokio::spawn`'d task) and uses
  `tokio::sync::watch` for change notifications.
```

- [ ] **Step 4: Verify docs build**

Run: `cargo doc --all-features --no-deps`
Expected: succeeds (warnings about intra-doc links to cfg-gated items when building
without `--all-features` are expected and pre-existing — e.g. `Toml`/`Json` already do
this — not a regression).

- [ ] **Step 5: Run the full verification matrix one more time**

Run: `cargo build --workspace --all-features --tests`
Run: `cargo test --workspace --all-features`
Run: `cargo fmt --all -- --check`
Run: `cargo clippy --workspace --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features toml --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features json --all-targets -- -D warnings`
Run: `cargo clippy --workspace --no-default-features --features tokio --all-targets -- -D warnings`
Run: `cargo clippy --workspace --all-features --all-targets -- -D warnings`
Run: `cargo audit --file Cargo.lock`
Expected: all clean, 0 vulnerabilities.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml src/lib.rs README.md
git commit -m "Add tokio to the CI feature matrix and crate/README docs"
```

---

## Post-merge (not part of this plan's tasks — done directly, not by a subagent)

Once this branch's PR is open and its own CI has run at least once, add `clippy
(tokio)` to `main`'s branch-protection required status checks (mirroring how `audit`
was added after the CI-setup PR first ran):

```bash
gh api repos/bgallet/dyn-properties/branches/main/protection/required_status_checks -X PATCH --input - <<'EOF'
{
  "strict": true,
  "contexts": ["fmt", "clippy (none)", "clippy (toml)", "clippy (json)", "clippy (tokio)", "clippy (all)", "test", "audit"]
}
EOF
```
