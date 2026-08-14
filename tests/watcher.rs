use dyn_properties::{DynProperties, PropertyWatcher};
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

    let watcher = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(watcher.load().port, 9000);
}

#[tokio::test]
async fn start_fails_on_invalid_initial_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 99999").unwrap();

    let result = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_secs(60)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn reload_picks_up_valid_changes() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "port = 9000").unwrap();

    let watcher = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_millis(50))
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

    let watcher = PropertyWatcher::<AppConfig>::start(file.path(), Duration::from_millis(50))
        .await
        .unwrap();

    std::fs::write(file.path(), "port = 99999").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(watcher.load().port, 9000);
    assert!(logs_contain("reload failed"));
}
