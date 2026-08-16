use dyn_properties::{DynProperties, Format, PropertyWatcher, Toml};
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
    // Parses fine as a u16 but violates #[range(min = 1, max = 65535)]; this must be
    // rejected by `Validate`, not by TOML deserialization (see `Error::Validation`).
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

    // Parses fine as a u16 but violates #[range(min = 1, max = 65535)]; this must be
    // rejected by `Validate` during the reload tick, not by TOML deserialization.
    std::fs::write(file.path(), "port = 0").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload failed"));
    // Confirm the discard was driven by the Validate path (Error::Validation's Display
    // is "<field_path>: <reason>"), not by an unrelated I/O or TOML-parse failure.
    assert!(logs_contain("port: 0 is out of range"));
}

const PANIC_SENTINEL: &str = "__PANIC__";

/// A `Format` that behaves exactly like [`Toml`] except it deliberately panics if the
/// raw file bytes contain a sentinel marker. Used to prove `PropertyWatcher` survives an
/// arbitrary panic during a reload tick's `Format::parse` call — e.g. a bug in a
/// third-party `Format` implementation — now that a malformed `#[range]`/`#[default]`
/// duration literal (the previous way this test triggered a panic) is a compile error
/// instead of a runtime one and can no longer be used to construct this scenario.
struct PanicOnSentinel;

impl Format for PanicOnSentinel {
    type Error = <Toml as Format>::Error;

    fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Self::Error> {
        let text = String::from_utf8_lossy(bytes);
        if text.contains(PANIC_SENTINEL) {
            panic!("PanicOnSentinel: deliberate panic triggered by test sentinel");
        }
        Toml::parse(bytes)
    }
}

#[tokio::test]
#[traced_test]
async fn reload_survives_a_panic_inside_a_tick() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, PanicOnSentinel>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(watcher.load().port, 9000);

    // Triggers PanicOnSentinel::parse's deliberate panic during the next reload tick.
    std::fs::write(file.path(), format!("port = 9000 # {PANIC_SENTINEL}\n")).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The watcher task must have survived: `load()` still returns the last-good value,
    // not a hang or a propagated panic, and the panic was logged rather than swallowed.
    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload tick panicked"));

    // A subsequent tick against a file without the sentinel proves the watcher's loop is
    // still alive and reloading, not permanently dead.
    std::fs::write(file.path(), "port = 9500\n").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9500);
}
