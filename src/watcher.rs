use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use arc_swap::{ArcSwap, Guard};

use crate::{Error, Format, Validate};

/// A config file loaded once and then polled on a background thread, exposing the latest
/// successfully-parsed-and-validated value via [`load`](Self::load).
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
pub struct PropertyWatcher<T, F> {
    inner: Arc<ArcSwap<T>>,
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
    pub fn start(path: impl Into<PathBuf>, interval: std::time::Duration) -> Result<Self, Error> {
        let path = path.into();
        let initial = load_and_validate::<T, F>(&path)?;
        let inner = Arc::new(ArcSwap::new(Arc::new(initial)));

        let watcher_inner = Arc::clone(&inner);
        let watch_path = path.clone();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        // Carries the caller's current tracing span onto the background thread, so
        // reload logs stay correlated with whatever context `start` was called from
        // (a fresh OS thread otherwise starts with no span of its own).
        let span = tracing::Span::current();
        thread::spawn(move || {
            let _entered = span.enter();
            loop {
                // Doubles as the poll interval's sleep and the stop signal: a normal
                // timeout means "reload now", while a disconnect (the watcher was
                // dropped) wakes this up immediately instead of waiting out the rest
                // of the interval.
                match stop_rx.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                match load_and_validate::<T, F>(&watch_path) {
                    Ok(value) => watcher_inner.store(Arc::new(value)),
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

        Ok(PropertyWatcher {
            inner,
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
}

fn load_and_validate<T, F>(path: &Path) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned + Validate,
    F: Format,
{
    let bytes = std::fs::read(path)?;
    let value: T = F::parse(&bytes).map_err(|e| Error::Parse(Box::new(e)))?;
    value.validate()?;
    Ok(value)
}
