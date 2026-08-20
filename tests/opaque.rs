use dyn_properties::{DynProperties, Validate};
use std::collections::HashMap;

#[derive(DynProperties, Debug)]
struct AppConfig {
    #[range(min = 1, max = 65535)]
    #[default(8080)]
    port: u16,

    #[default(vec!["dev".to_string()])]
    tags: Vec<String>,

    labels: HashMap<String, String>,
}

#[test]
fn vec_field_defaults_via_custom_default_expression() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.tags, vec!["dev".to_string()]);
}

#[test]
fn map_field_defaults_to_empty_when_no_default_given() {
    let cfg = AppConfig::default();
    assert!(cfg.labels.is_empty());
}

#[test]
fn vec_field_absent_from_file_uses_default() {
    let cfg: AppConfig = toml::from_str("port = 9000").unwrap();
    assert_eq!(cfg.tags, vec!["dev".to_string()]);
}

#[test]
fn vec_field_present_in_file_overrides_default() {
    let cfg: AppConfig = toml::from_str(r#"tags = ["prod", "eu"]"#).unwrap();
    assert_eq!(cfg.tags, vec!["prod".to_string(), "eu".to_string()]);
}

#[test]
fn map_field_present_in_file_deserializes() {
    let cfg: AppConfig = toml::from_str(
        r#"
        [labels]
        team = "payments"
        tier = "critical"
        "#,
    )
    .unwrap();
    assert_eq!(cfg.labels.get("team"), Some(&"payments".to_string()));
    assert_eq!(cfg.labels.get("tier"), Some(&"critical".to_string()));
}

#[test]
fn other_fields_still_validate_normally_alongside_opaque_fields() {
    let cfg: AppConfig = toml::from_str("port = 0").unwrap();
    assert!(cfg.validate().is_err());

    let cfg: AppConfig = toml::from_str("port = 9000").unwrap();
    assert!(cfg.validate().is_ok());
}

// A plain type that is deliberately NOT #[derive(DynProperties)] — proving the
// #[opaque] escape hatch works for arbitrary non-DynProperties types, not just the
// auto-detected standard collections.
#[derive(serde::Deserialize, Default, Debug, PartialEq, Eq)]
enum LogFormat {
    #[default]
    Text,
    Json,
}

#[derive(DynProperties, Debug)]
struct LoggingConfig {
    #[opaque]
    format: LogFormat,
}

#[test]
fn opaque_attribute_allows_an_arbitrary_non_dynproperties_type() {
    let cfg = LoggingConfig::default();
    assert_eq!(cfg.format, LogFormat::Text);

    let cfg: LoggingConfig = toml::from_str(r#"format = "Json""#).unwrap();
    assert_eq!(cfg.format, LogFormat::Json);
}
