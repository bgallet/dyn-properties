use dyn_properties::DynProperties;
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

#[test]
fn empty_toml_uses_all_defaults() {
    let cfg: DbConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 5432);
    assert_eq!(cfg.pool.idle, 5);
    assert_eq!(cfg.pool.active, 2);
    assert_eq!(cfg.description, None);
}

#[test]
fn overriding_top_level_field_keeps_other_defaults() {
    let cfg: DbConfig = toml::from_str("port = 9999").unwrap();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 9999);
}

#[test]
fn partial_nested_table_only_overrides_specified_subfield() {
    let cfg: DbConfig = toml::from_str("[pool]\nidle = 40").unwrap();
    assert_eq!(cfg.pool.idle, 40);
    assert_eq!(cfg.pool.active, 2);
}

#[test]
fn option_field_present_becomes_some() {
    let cfg: DbConfig = toml::from_str(r#"description = "primary db""#).unwrap();
    assert_eq!(cfg.description, Some("primary db".to_string()));
}

#[test]
fn option_field_absent_stays_none() {
    let cfg: DbConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.description, None);
}

#[test]
fn fully_specified_toml_overrides_everything() {
    let cfg: DbConfig = toml::from_str(
        r#"
        host = "db.internal"
        port = 6543

        [pool]
        idle = 10
        active = 4
        "#,
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
    let cfg: DbConfig = toml::from_str("port = 9999\ntypo_field = 1").unwrap();
    assert_eq!(cfg.port, 9999);
    assert_eq!(cfg.host, "localhost");
    assert!(logs_contain("ignoring unknown field"));
    assert!(logs_contain("DbConfig"));
    assert!(logs_contain("typo_field"));
}

#[test]
#[traced_test]
fn unknown_nested_field_is_logged_against_the_nested_struct_name() {
    let cfg: DbConfig = toml::from_str("[pool]\nidle = 40\nbogus = 1").unwrap();
    assert_eq!(cfg.pool.idle, 40);
    assert!(logs_contain("ignoring unknown field"));
    assert!(logs_contain("PoolConfig"));
    assert!(logs_contain("bogus"));
}

#[test]
#[traced_test]
fn no_warning_when_every_field_is_known() {
    let _cfg: DbConfig = toml::from_str("port = 9999").unwrap();
    assert!(!logs_contain("ignoring unknown field"));
}
