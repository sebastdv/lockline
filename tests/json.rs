use lockline::report::render_json;
use lockline::{Config, MigrationUnit, Report, analyze};

fn report() -> Report {
    let units = [MigrationUnit {
        source: Default::default(),
        name: "migrations/0007_add_email_index.sql".into(),
        sql: "CREATE INDEX idx_users_email ON users (email);\n\
              -- lockline:ignore missing-lock-timeout runs off-peak\n\
              ALTER TABLE users ADD COLUMN nickname text;\n\
              GRANT SELECT ON users TO app;\n"
            .into(),
    }];
    analyze(&units, &Config::default())
}

#[test]
fn json_snapshot() {
    insta::assert_snapshot!(render_json(&report()));
}

#[test]
fn json_is_valid_and_stable() {
    let value: serde_json::Value = serde_json::from_str(&render_json(&report())).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["summary"]["errors"], 1);
    assert_eq!(value["summary"]["suppressed"], 1);
    assert_eq!(value["summary"]["not_analyzed"], 1);
    let file = &value["files"][0];
    assert_eq!(file["path"], "migrations/0007_add_email_index.sql");
    let first = &file["statements"][0];
    assert_eq!(first["effects"][0]["lock"], "SHARE");
    assert_eq!(first["effects"][0]["cost"], "scan");
    assert_eq!(first["effects"][0]["blocks_writes"], true);
    assert_eq!(
        first["findings"][0]["rule"],
        "create-index-not-concurrently"
    );
    assert_eq!(first["findings"][0]["severity"], "error");
    assert_eq!(first["findings"][0]["line"], 1);
    assert_eq!(
        file["statements"][1]["suppressed"][0]["reason"],
        "runs off-peak"
    );
    assert_eq!(file["statements"][2]["analyzed"], false);
    assert_eq!(
        file["transactions"][0]["ends_after_line"],
        serde_json::Value::Null
    );
    assert!(render_json(&report()).ends_with("}\n"));
}
