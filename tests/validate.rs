use dyn_properties::{DynProperties, Validate};
use std::time::Duration;

#[derive(DynProperties)]
struct PoolConfig {
    #[range(min = 0, max = 50)]
    idle: u32,
}

#[derive(DynProperties)]
struct DbConfig {
    #[len(min = 3, max = 64)]
    host: String,

    #[range(min = 1, max = 65535)]
    port: u32,

    #[range(min = "100ms", max = "30s")]
    #[default("5s")]
    connect_timeout: Duration,

    #[range(min = 1, max = 100)]
    max_conns: Option<u32>,

    pool: PoolConfig,
}

fn valid_config() -> DbConfig {
    DbConfig {
        host: "localhost".to_string(),
        port: 5432,
        connect_timeout: dyn_properties::parse_duration("5s").unwrap(),
        max_conns: Some(10),
        pool: PoolConfig { idle: 5 },
    }
}

#[test]
fn valid_values_pass() {
    assert!(valid_config().validate().is_ok());
}

#[test]
fn boundary_values_pass() {
    let mut cfg = valid_config();
    cfg.port = 65535;
    cfg.host = "a".repeat(64);
    cfg.connect_timeout = dyn_properties::parse_duration("30s").unwrap();
    assert!(cfg.validate().is_ok());
}

#[test]
fn numeric_out_of_range_fails_with_field_path() {
    let mut cfg = valid_config();
    cfg.port = 70000;
    let err = cfg.validate().unwrap_err();
    match err {
        dyn_properties::Error::Validation { field_path, .. } => assert_eq!(field_path, "port"),
        _ => panic!("expected Validation error"),
    }
}

#[test]
fn string_too_short_fails() {
    let mut cfg = valid_config();
    cfg.host = "ab".to_string();
    assert!(cfg.validate().is_err());
}

#[test]
fn duration_out_of_range_fails() {
    let mut cfg = valid_config();
    cfg.connect_timeout = dyn_properties::parse_duration("1h").unwrap();
    assert!(cfg.validate().is_err());
}

#[test]
fn option_bound_checked_only_when_some() {
    let mut cfg = valid_config();
    cfg.max_conns = None;
    assert!(cfg.validate().is_ok());

    cfg.max_conns = Some(1000);
    assert!(cfg.validate().is_err());
}

#[test]
fn nested_struct_validated_with_dot_joined_field_path() {
    let mut cfg = valid_config();
    cfg.pool.idle = 999;
    let err = cfg.validate().unwrap_err();
    match err {
        dyn_properties::Error::Validation { field_path, .. } => assert_eq!(field_path, "pool.idle"),
        _ => panic!("expected Validation error"),
    }
}

#[derive(DynProperties)]
struct Level3 {
    #[range(min = 0, max = 10)]
    deep: u32,
}

#[derive(DynProperties)]
struct Level2 {
    level3: Level3,
}

#[derive(DynProperties)]
struct Level1 {
    level2: Level2,
}

#[derive(DynProperties)]
struct RootConfig {
    level1: Level1,
}

#[test]
fn three_levels_of_nesting_compose_a_dot_joined_field_path() {
    let mut cfg = RootConfig {
        level1: Level1 {
            level2: Level2 {
                level3: Level3 { deep: 5 },
            },
        },
    };
    assert!(cfg.validate().is_ok());

    cfg.level1.level2.level3.deep = 999;
    let err = cfg.validate().unwrap_err();
    match err {
        dyn_properties::Error::Validation { field_path, .. } => {
            assert_eq!(field_path, "level1.level2.level3.deep")
        }
        _ => panic!("expected Validation error"),
    }
}
