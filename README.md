# dyn-properties

Reads a config file — TOML, JSON, or your own `Format` — into a validated,
hot-reloadable struct.

## Why

A surprising number of the "constants" in a running service aren't really
constant — they're operational knobs that need to move without a restart:

- **Rate limiting.** The request-rate ceiling and the blacklisted-user list
  both need to change in response to what's happening right now, not on the
  next deploy.
- **Connection pool sizing.** Bumping a database pool's size during a
  traffic spike — or shrinking it when a downstream dependency is
  struggling — is often the difference between a smooth recovery and an
  incident.
- **Short-lived certificates.** A TLS certificate that rotates every few
  hours needs to be picked up without dropping connections; restarting the
  process to reload it isn't an option.
- **Circuit-breaker thresholds.** Tightening a failure-rate threshold or
  retry backoff during an incident — and loosening it again once the
  dependency recovers — shouldn't need a deploy either way.
- **Feature-flag rollout percentage.** Dialing a canary from 1% to 100%,
  or back to 0% at the first sign of trouble, is inherently a live
  operation.

`dyn-properties` lets values like these live in a config file instead of in
code: edit the file, and the next reload picks up the change — validated
the same way every time, with a failed reload keeping the last good value
in place rather than taking the service down.

## Usage

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

`Vec<T>`, `HashMap`, `BTreeMap`, `HashSet`, `BTreeSet`, and `VecDeque` fields
are recognized automatically — deserialized and defaulted (`#[default(...)]`
takes any expression of the field's type, e.g. `vec!["dev".to_string()]`),
but not recursively validated. Any other field type that isn't itself
`#[derive(DynProperties)]` needs the same treatment but can't be recognized
by name — mark it `#[opaque]` explicitly.

## Cargo features

Neither format is enabled by default — enable exactly the one(s) you need:

- `toml` — TOML config files.
- `json` — JSON config files.
- `tokio` — adds a tokio-native `PropertyWatcher` under `dyn_properties::tokio` that
  spawns no OS thread (background refresh runs as a `tokio::spawn`'d task) and uses
  `tokio::sync::watch` for change notifications.

The two format features can be enabled together. With neither enabled, the
`Format` trait itself is still available — implement it for your own format
(YAML, RON, ...) and use `PropertyWatcher<T, YourFormat>` without depending
on `toml` or `serde_json` at all.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
