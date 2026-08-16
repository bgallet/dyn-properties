# dyn-properties

Reads a config file — TOML, JSON, or your own `Format` — into a validated,
hot-reloadable struct.

```rust
use dyn_properties::{DynProperties, PropertyWatcher, Toml};
use std::sync::Arc;
use std::time::Duration;

#[derive(DynProperties)]
struct AppConfig {
    #[len(min = 3, max = 64)]
    #[default("localhost")]
    host: String,

    #[range(min = 1, max = 65535)]
    #[default(8080)]
    port: u16,

    #[range(min = "100ms", max = "30s")]
    #[default("5s")]
    request_timeout: Duration,
}

fn run() -> Result<(), dyn_properties::Error> {
    let watcher = PropertyWatcher::<AppConfig, Toml>::start(
        "config.toml",
        Duration::from_secs(30),
    )?;

    // Short-lived, same-thread read:
    let port = watcher.load().port;

    // Passing the config to another thread or an async task: clone the Arc
    // out first.
    let cfg: Arc<AppConfig> = Arc::clone(&watcher.load());
    some_other_fn(cfg);
    Ok(())
}
```

Only fields that differ from their `#[default(...)]` need to be present in
the config file — everything else falls back to its default. Bounds
(`#[range]`, `#[len]`) are validated on every load, including reloads, and a
failed reload keeps the previously-loaded value in place rather than
propagating the error.

## Cargo features

Neither format is enabled by default — enable exactly the one(s) you need:

- `toml` — TOML config files.
- `json` — JSON config files.

Both can be enabled together. With neither enabled, the `Format` trait
itself is still available — implement it for your own format (YAML, RON,
...) and use `PropertyWatcher<T, YourFormat>` without depending on `toml` or
`serde_json` at all.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
