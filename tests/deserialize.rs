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

// A required field nested inside a struct is only enforced once the file provides that
// section at all: if `[required_secret]` were entirely omitted, `helper.required_secret`
// would be `None` and fall back to `SecretConfig::default()` directly, which — being
// `Default`, not `Deserialize` — never runs `SecretConfig`'s own required-field check.
// Marking the nested field itself #[required] closes that gap by forcing the section's
// presence, at which point `SecretConfig::deserialize` (and its own check) always runs.
#[derive(DynProperties, Debug)]
struct ApiConfigWithRequiredSection {
    #[required]
    required_secret: SecretConfig,
}

#[test]
fn required_field_present_loads_successfully() {
    let cfg: ApiConfig = toml::from_str(
        r#"
        api_key = "abc123"

        [secret]
        password = "hunter2"
        "#,
    )
    .unwrap();
    assert_eq!(cfg.api_key, "abc123");
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.secret.password, "hunter2");
}

#[test]
fn required_field_missing_fails_to_deserialize() {
    let result: Result<ApiConfig, _> = toml::from_str(
        r#"
        port = 9000

        [secret]
        password = "hunter2"
        "#,
    );
    let err = result.unwrap_err().to_string();
    assert!(err.contains("required"), "error was: {err}");
    assert!(err.contains("api_key"), "error was: {err}");
    assert!(err.contains("ApiConfig"), "error was: {err}");
}

#[test]
fn required_field_missing_within_a_present_nested_table_fails() {
    // `[secret]` is present, but its own required `password` is not: SecretConfig's own
    // Deserialize (and required-field check) runs regardless of whether the *outer*
    // field itself is marked #[required].
    let result: Result<ApiConfig, _> = toml::from_str("api_key = \"abc123\"\n[secret]\n");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("required"), "error was: {err}");
    assert!(err.contains("password"), "error was: {err}");
    assert!(err.contains("SecretConfig"), "error was: {err}");
}

#[test]
fn required_nested_field_rejects_a_fully_omitted_section() {
    // required_secret is #[required] at the outer level too, so omitting the whole
    // `[required_secret]` section is caught there — see the comment on
    // ApiConfigWithRequiredSection above for why that's necessary.
    let result: Result<ApiConfigWithRequiredSection, _> = toml::from_str("");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("required"), "error was: {err}");
    assert!(err.contains("required_secret"), "error was: {err}");
}
