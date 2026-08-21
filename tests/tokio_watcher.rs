use dyn_properties::tokio::PropertyWatcher;
use dyn_properties::{DynProperties, Toml};
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
    writeln!(file, "port = 0").unwrap();

    let result =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).await;
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

    let result =
        PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_secs(60)).await;
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

    std::fs::write(file.path(), "port = 0").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload failed"));
    assert!(logs_contain("port: 0 is out of range"));
}

#[tokio::test]
async fn subscribe_delivers_the_latest_value() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    std::fs::write(file.path(), "port = 9500").unwrap();
    tokio::time::timeout(Duration::from_secs(5), rx.changed())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(rx.borrow().port, 9500);
}

#[tokio::test]
async fn subscribe_coalesces_multiple_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    std::fs::write(file.path(), "port = 9100").unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    std::fs::write(file.path(), "port = 9200").unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    std::fs::write(file.path(), "port = 9300").unwrap();
    // Give the 50ms-interval background task a tick to actually observe this last
    // write before we check: `changed()` correctly returns as soon as any pending
    // change exists rather than queuing, so without this the assertion below can
    // race and observe 9200 (the second write) instead of 9300. See the equivalent
    // settle sleep and comment in tests/watcher.rs's
    // `subscriber_only_sees_the_latest_value_after_multiple_changes`.
    tokio::time::sleep(Duration::from_millis(150)).await;

    tokio::time::timeout(Duration::from_secs(5), rx.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rx.borrow().port, 9300);
}

#[tokio::test]
async fn subscriber_gets_no_notification_for_a_byte_identical_rewrite() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    // Same bytes as the file already has; must not be treated as a change.
    std::fs::write(file.path(), "port = 9000").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(50), rx.changed())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn dropping_the_watcher_ends_the_subscription() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig, Toml>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();
    let mut rx = watcher.subscribe();

    drop(watcher);

    assert!(rx.changed().await.is_err());
}
