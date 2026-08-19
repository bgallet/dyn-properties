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
                        // `send_replace` (unlike `send`) updates the stored value even
                        // when there are currently zero live receivers — `load()` reads
                        // through the `Sender` itself, so a caller that only ever calls
                        // `load()` and never `subscribe()`s must still observe reloads.
                        tx.send_replace(Arc::new(value));
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
