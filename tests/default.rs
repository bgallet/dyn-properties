use dyn_properties::DynProperties;

#[derive(DynProperties)]
struct PoolConfig {
    #[range(min = 0, max = 50)]
    #[default(5)]
    idle: u32,

    #[range(min = 1, max = 20)]
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

    #[duration_range(min = "100ms", max = "30s")]
    #[default("5s")]
    connect_timeout: dyn_properties::Duration,

    #[range(min = 1, max = 100)]
    max_conns: u32,

    pool: PoolConfig,

    description: Option<String>,

    #[default("primary")]
    label: Option<String>,
}

#[test]
fn default_uses_attribute_values() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.port, 5432);
    assert_eq!(*cfg.connect_timeout, std::time::Duration::from_secs(5));
}

#[test]
fn default_falls_back_to_type_default_without_attribute() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.max_conns, 0);
    assert_eq!(cfg.description, None);
}

#[test]
fn nested_struct_uses_its_own_generated_default() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.pool.idle, 5);
    assert_eq!(cfg.pool.active, 0);
}

#[test]
fn option_field_with_default_attribute_becomes_some() {
    let cfg = DbConfig::default();
    assert_eq!(cfg.label, Some("primary".to_string()));
}

#[derive(DynProperties)]
struct TimeoutConfig {
    #[duration_range(min = "0s", max = "1h")]
    idle_timeout: dyn_properties::Duration,
}

#[test]
fn unannotated_duration_field_defaults_to_zero() {
    let cfg = TimeoutConfig::default();
    assert!((*cfg.idle_timeout).is_zero());
}
