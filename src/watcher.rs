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
///
/// Config files should be updated atomically (write to a temp file in the same
/// directory, then rename over the target) rather than truncated in place — a poll
/// tick that reads a mid-write, momentarily-empty file will parse and validate
/// successfully as "everything defaulted" under this crate's default-overlay design,
/// silently replacing good config rather than failing loudly.
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
