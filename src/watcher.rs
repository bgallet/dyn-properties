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
