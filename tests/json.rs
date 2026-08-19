use dyn_properties::{DynProperties, Format, Json};
use std::time::Duration;
use tracing_test::traced_test;

#[derive(DynProperties)]
struct PoolConfig {
    #[range(min = 0, max = 50)]
    #[default(5)]
    idle: u32,

    #[range(min = 1, max = 20)]
    #[default(2)]
    active: u32,
}

#[derive(DynProperties)]
struct DbConfig {
    #[len(min = 3, max = 64)]
    #[default("localhost")]
    host: String,

    #[range(min = 1, max = 65535)]
    #[default(5432)]
    port: u16,

    pool: PoolConfig,

    description: Option<String>,
}

#[derive(DynProperties)]
struct TimeoutConfig {
    #[range(min = "100ms", max = "1h")]
    grace_period: Option<Duration>,
}

#[test]
fn null_option_duration_becomes_none() {
    let cfg: TimeoutConfig = Json::parse(br#"{"grace_period": null}"#).unwrap();
    assert_eq!(cfg.grace_period, None);
}

#[test]
fn present_option_duration_still_parses() {
    let cfg: TimeoutConfig = Json::parse(br#"{"grace_period": "5s"}"#).unwrap();
    assert_eq!(cfg.grace_period, Some(Duration::from_secs(5)));
}

#[test]
fn empty_json_object_uses_all_defaults() {
    let cfg: DbConfig = Json::parse(b"{}").unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 5432);
    assert_eq!(cfg.pool.idle, 5);
    assert_eq!(cfg.pool.active, 2);
    assert_eq!(cfg.description, None);
}

#[test]
fn overriding_top_level_field_keeps_other_defaults() {
    let cfg: DbConfig = Json::parse(br#"{"port": 9999}"#).unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 9999);
}

#[test]
fn partial_nested_object_only_overrides_specified_subfield() {
    let cfg: DbConfig = Json::parse(br#"{"pool": {"idle": 40}}"#).unwrap();
    assert_eq!(cfg.pool.idle, 40);
    assert_eq!(cfg.pool.active, 2);
}

#[test]
fn option_field_present_becomes_some() {
    let cfg: DbConfig = Json::parse(br#"{"description": "primary db"}"#).unwrap();
    assert_eq!(cfg.description, Some("primary db".to_string()));
}

#[test]
fn option_field_absent_stays_none() {
    let cfg: DbConfig = Json::parse(b"{}").unwrap();
    assert_eq!(cfg.description, None);
}

#[test]
fn fully_specified_json_overrides_everything() {
    let cfg: DbConfig = Json::parse(
        br#"{
            "host": "db.internal",
            "port": 6543,
            "pool": { "idle": 10, "active": 4 }
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.host, "db.internal");
    assert_eq!(cfg.port, 6543);
    assert_eq!(cfg.pool.idle, 10);
    assert_eq!(cfg.pool.active, 4);
}

#[test]
#[traced_test]
fn unknown_top_level_field_is_logged_and_ignored() {
    let cfg: DbConfig = Json::parse(br#"{"port": 9999, "typo_field": 1}"#).unwrap();
    assert_eq!(cfg.port, 9999);
    assert_eq!(cfg.host, "localhost");
    assert!(logs_contain("ignoring unknown field"));
    assert!(logs_contain("DbConfig"));
    assert!(logs_contain("typo_field"));
}

#[test]
#[traced_test]
fn unknown_nested_field_is_logged_against_the_nested_struct_name() {
    let cfg: DbConfig = Json::parse(br#"{"pool": {"idle": 40, "bogus": 1}}"#).unwrap();
    assert_eq!(cfg.pool.idle, 40);
    assert!(logs_contain("ignoring unknown field"));
    assert!(logs_contain("PoolConfig"));
    assert!(logs_contain("bogus"));
}

#[test]
#[traced_test]
fn no_warning_when_every_field_is_known() {
    let _cfg: DbConfig = Json::parse(br#"{"port": 9999}"#).unwrap();
    assert!(!logs_contain("ignoring unknown field"));
}

#[derive(DynProperties, Debug)]
struct SecretConfig {
    #[required]
    password: String,
}

#[derive(DynProperties, Debug)]
struct ApiConfig {
    #[required]
    api_key: String,

    #[range(min = 1, max = 65535)]
    #[default(8080)]
    port: u16,

    secret: SecretConfig,
}

#[derive(DynProperties, Debug)]
struct ApiConfigWithRequiredSection {
    #[required]
    required_secret: SecretConfig,
}

#[test]
fn required_field_present_loads_successfully() {
    let cfg: ApiConfig =
        Json::parse(br#"{"api_key": "abc123", "secret": {"password": "hunter2"}}"#).unwrap();
    assert_eq!(cfg.api_key, "abc123");
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.secret.password, "hunter2");
}

#[test]
fn required_field_missing_fails_to_deserialize() {
    let result: Result<ApiConfig, _> =
        Json::parse(br#"{"port": 9000, "secret": {"password": "hunter2"}}"#);
    let err = result.unwrap_err().to_string();
    assert!(err.contains("required"), "error was: {err}");
    assert!(err.contains("api_key"), "error was: {err}");
    assert!(err.contains("ApiConfig"), "error was: {err}");
}

#[test]
fn required_field_missing_within_a_present_nested_object_fails() {
    let result: Result<ApiConfig, _> = Json::parse(br#"{"api_key": "abc123", "secret": {}}"#);
    let err = result.unwrap_err().to_string();
    assert!(err.contains("required"), "error was: {err}");
    assert!(err.contains("password"), "error was: {err}");
    assert!(err.contains("SecretConfig"), "error was: {err}");
}

#[test]
fn required_nested_field_rejects_a_fully_omitted_section() {
    let result: Result<ApiConfigWithRequiredSection, _> = Json::parse(b"{}");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("required"), "error was: {err}");
    assert!(err.contains("required_secret"), "error was: {err}");
}
