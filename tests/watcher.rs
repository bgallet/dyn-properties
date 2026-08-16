use dyn_properties::{DynProperties, PropertyWatcher, Toml};
use std::io::Write;
use std::time::Duration;
use tracing_test::traced_test;

#[derive(DynProperties)]
struct AppConfig {
    #[range(min = 1, max = 65535)]
    #[default(8080)]
    port: u16,
}

#[test]
fn start_loads_initial_values() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).unwrap();

    assert_eq!(watcher.load().port, 9000);
}

#[test]
fn start_fails_on_invalid_initial_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    // Parses fine as a u16 but violates #[range(min = 1, max = 65535)]; this must be
    // rejected by `Validate`, not by TOML deserialization (see `Error::Validation`).
    writeln!(file, "port = 0").unwrap();

    let result = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60));
    match result {
        Err(dyn_properties::Error::Validation { .. }) => {}
        Err(other) => panic!("expected Error::Validation, got {other:?}"),
        Ok(_) => panic!("expected start() to fail on out-of-range port"),
    }
}

#[test]
fn start_fails_with_error_parse_on_unparseable_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "not valid = [").unwrap();

    let result = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60));
    match result {
        Err(dyn_properties::Error::Parse(_)) => {}
        Err(other) => panic!("expected Error::Parse, got {other:?}"),
        Ok(_) => panic!("expected start() to fail on unparseable TOML"),
    }
}

#[test]
fn reload_picks_up_valid_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();
    assert_eq!(watcher.load().port, 9000);

    std::fs::write(file.path(), "port = 9500").unwrap();
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(watcher.load().port, 9500);
}

#[test]
#[traced_test]
fn reload_keeps_last_good_value_on_invalid_change_and_logs() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();

    // Parses fine as a u16 but violates #[range(min = 1, max = 65535)]; this must be
    // rejected by `Validate` during the reload tick, not by TOML deserialization.
    std::fs::write(file.path(), "port = 0").unwrap();
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload failed"));
    // Confirm the discard was driven by the Validate path (Error::Validation's Display
    // is "<field_path>: <reason>"), not by an unrelated I/O or TOML-parse failure.
    assert!(logs_contain("port: 0 is out of range"));
}
