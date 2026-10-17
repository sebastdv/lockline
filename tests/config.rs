use std::fs;

use lockline::config::{ConfigError, RuleLevel, find_config, load_config};
use lockline::model::Severity;

fn write_config(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lockline.toml");
    fs::write(&path, contents).unwrap();
    (dir, path)
}

#[test]
fn load_valid_config() {
    let (_dir, path) = write_config(
        r#"
transaction = "per-unit"
fail_on = "error"
exclude = ["migrations/legacy/**"]
[rules]
drop-object = "off"
missing-lock-timeout = "error"
"#,
    );
    let config = load_config(&path).unwrap();
    assert_eq!(config.transaction, Some(lockline::TransactionMode::PerUnit));
    assert_eq!(config.fail_on, Some(Severity::Error));
    assert_eq!(config.exclude, ["migrations/legacy/**"]);
    assert_eq!(config.rules["drop-object"], RuleLevel::Off);
    assert_eq!(config.rules["missing-lock-timeout"], RuleLevel::Error);
}

#[test]
fn unknown_rule_is_an_error() {
    let (_dir, path) = write_config("[rules]\ncreate-index-not-concurently = \"off\"\n");
    let err = load_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::UnknownRule { .. }));
    let message = err.to_string();
    assert!(message.contains("lockline.toml"), "{message}");
    assert!(
        message.contains("unknown rule \"create-index-not-concurently\""),
        "{message}"
    );
}

#[test]
fn unknown_key_is_an_error() {
    let (_dir, path) = write_config("fail_onn = \"error\"\n");
    let err = load_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid { .. }));
    assert!(err.to_string().contains("fail_onn"), "{err}");
}

#[test]
fn bad_values_are_errors() {
    let (_dir, path) = write_config("transaction = \"sometimes\"\n");
    assert!(matches!(
        load_config(&path).unwrap_err(),
        ConfigError::Invalid { .. }
    ));
}

#[test]
fn find_config_walks_up() {
    let (dir, path) = write_config("");
    let nested = dir.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(find_config(&nested), Some(path));
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(find_config(empty.path()), None);
}

#[test]
fn invalid_value_reports_position() {
    let (_dir, path) = write_config("[rules]\ndrop-object = \"of\"\n");
    let message = load_config(&path).unwrap_err().to_string();
    assert!(message.contains("line 2"), "{message}");
    assert!(message.contains("drop-object"), "{message}");
}
