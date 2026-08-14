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
