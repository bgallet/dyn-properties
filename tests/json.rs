use dyn_properties::{DynProperties, Json, Format};

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
