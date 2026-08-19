use dyn_properties::{DynProperties, PropertyWatcher, Toml};
use std::io::Write;
use std::sync::mpsc;
use std::thread;
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

#[test]
#[traced_test]
fn persistently_invalid_unchanging_file_logs_only_once() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(30)).unwrap();

    std::fs::write(file.path(), "port = 0").unwrap();
    // Several poll intervals elapse while the file stays broken and unchanged; the
    // byte-comparison gate should treat this identical-but-still-broken content as
    // "already handled" after the first tick, instead of re-attempting (and re-logging)
    // on every subsequent tick.
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(watcher.load().port, 9000);
    logs_assert(|lines: &[&str]| {
        let count = lines
            .iter()
            .filter(|line| line.contains("reload failed"))
            .count();
        if count == 1 {
            Ok(())
        } else {
            Err(format!(
                "expected exactly 1 \"reload failed\" log line, got {count}"
            ))
        }
    });
}

#[test]
fn subscriber_receives_new_value_on_change() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();
    let mut subscription = watcher.subscribe();

    std::fs::write(file.path(), "port = 9500").unwrap();

    let received = subscription
        .wait_for_change_timeout(Duration::from_secs(5))
        .expect("expected a change notification");
    assert_eq!(received.port, 9500);
}

#[test]
fn subscriber_gets_no_notification_for_a_byte_identical_rewrite() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();
    let mut subscription = watcher.subscribe();

    // Same bytes as the file already has; must not be treated as a change.
    std::fs::write(file.path(), "port = 9000").unwrap();
    std::thread::sleep(Duration::from_millis(200));

    assert!(
        subscription
            .wait_for_change_timeout(Duration::from_millis(50))
            .is_none()
    );
}

#[test]
fn subscriber_gets_no_notification_before_subscribing() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();

    // Change happens before subscribe() is ever called.
    std::fs::write(file.path(), "port = 9500").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(watcher.load().port, 9500);

    let mut subscription = watcher.subscribe();
    assert!(
        subscription
            .wait_for_change_timeout(Duration::from_millis(50))
            .is_none()
    );
}

#[test]
fn multiple_subscribers_all_receive_the_same_change() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();
    let mut sub_a = watcher.subscribe();
    let mut sub_b = watcher.subscribe();

    std::fs::write(file.path(), "port = 9500").unwrap();

    assert_eq!(
        sub_a
            .wait_for_change_timeout(Duration::from_secs(5))
            .unwrap()
            .port,
        9500
    );
    assert_eq!(
        sub_b
            .wait_for_change_timeout(Duration::from_secs(5))
            .unwrap()
            .port,
        9500
    );
}

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
    // Give the 50ms-interval background thread a tick to actually observe this last
    // write before we check: wait_for_change_timeout correctly returns as soon as any
    // generation change is pending rather than queuing, so without this the assertion
    // below can race and observe 9200 (the second write) instead of 9300.
    std::thread::sleep(Duration::from_millis(150));

    // A single wait_for_change call coalesces all three intervening changes into the
    // latest value — no backlog of 9100/9200 to drain first.
    let received = subscription
        .wait_for_change_timeout(Duration::from_secs(5))
        .expect("expected a change notification");
    assert_eq!(received.port, 9300);
}

#[test]
fn dropping_the_watcher_ends_the_subscription() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50)).unwrap();
    let mut subscription = watcher.subscribe();

    // Use the unbounded wait (not the timeout variant) on a separate thread, so this
    // test genuinely exercises a *blocked* waiter observing `close()`'s
    // `notify_all()` — the timeout variant collapses "closed" and "timed out" into
    // the same `None`, so it would pass even if `close()` were entirely removed.
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = subscription.wait_for_change();
        tx.send(result).unwrap();
    });

    // Give the spawned thread time to actually enter the blocking wait before we drop.
    thread::sleep(Duration::from_millis(50));

    drop(watcher);

    // Bounded so a regression fails the test cleanly instead of hanging the suite.
    let result = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.is_none());
}

#[tokio::test]
#[cfg(feature = "tokio")]
#[traced_test]
async fn starting_threaded_watcher_inside_a_tokio_runtime_logs_a_warning() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let _watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).unwrap();

    assert!(logs_contain(
        "consider dyn_properties::tokio::PropertyWatcher"
    ));
}

#[test]
#[cfg(feature = "tokio")]
#[traced_test]
fn starting_threaded_watcher_outside_a_tokio_runtime_does_not_warn() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let _watcher =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).unwrap();

    assert!(!logs_contain(
        "consider dyn_properties::tokio::PropertyWatcher"
    ));
}
