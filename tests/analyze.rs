use lockline::model::{Analysis, Severity};
use lockline::{Config, Counts, MigrationUnit, TransactionMode, UnitReport, analyze};

fn run(sql: &str, mode: TransactionMode) -> UnitReport {
    let units = [MigrationUnit {
        name: "m.sql".into(),
        sql: sql.into(),
    }];
    let mut report = analyze(&units, &Config { transaction: mode });
    report.units.remove(0)
}

fn rules(unit: &UnitReport) -> Vec<&'static str> {
    unit.findings
        .iter()
        .chain(unit.statements.iter().flat_map(|s| s.findings.iter()))
        .map(|f| f.rule)
        .collect()
}

#[test]
fn index_on_existing_table_is_an_error() {
    let unit = run("CREATE INDEX i ON users (email);", TransactionMode::PerUnit);
    let findings = &unit.statements[0].findings;
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, "create-index-not-concurrently");
    assert_eq!(findings[0].severity, Severity::Error);
    assert!(findings[0].message.contains("\"users\""));
    let help = findings[0].help.as_deref().unwrap();
    assert!(
        help.contains("CREATE INDEX CONCURRENTLY i ON users"),
        "{help}"
    );
    assert!(
        help.contains("cannot run inside a transaction block"),
        "{help}"
    );
}

#[test]
fn index_on_table_created_in_same_unit_is_fine() {
    let unit = run(
        "CREATE TABLE users (id int); CREATE INDEX i ON users (id);",
        TransactionMode::PerUnit,
    );
    assert!(rules(&unit).is_empty());
    assert!(unit.statements[1].new_table);
    assert!(!unit.statements[0].new_table);
}

#[test]
fn quoted_and_schema_names_match_like_postgres() {
    let same = run(
        "CREATE TABLE public.users (id int); CREATE INDEX a ON users (id);",
        TransactionMode::PerUnit,
    );
    assert!(rules(&same).is_empty());
    let different = run(
        "CREATE TABLE \"Users\" (id int); CREATE INDEX b ON users (id);",
        TransactionMode::PerUnit,
    );
    assert_eq!(rules(&different), ["create-index-not-concurrently"]);
}

#[test]
fn concurrently_in_per_unit_mode_fails() {
    let sql = "CREATE INDEX CONCURRENTLY i ON users (email);";
    let unit = run(sql, TransactionMode::PerUnit);
    let findings = &unit.statements[0].findings;
    assert_eq!(rules(&unit), ["cannot-run-in-transaction"]);
    assert!(
        findings[0]
            .message
            .starts_with("CREATE INDEX CONCURRENTLY cannot run")
    );
    assert!(
        findings[0]
            .help
            .as_deref()
            .unwrap()
            .contains("$withinTransaction = false")
    );
    assert!(rules(&run(sql, TransactionMode::None)).is_empty());
}

#[test]
fn explicit_commit_ends_transaction() {
    let committed = "BEGIN; ALTER TABLE users ADD COLUMN x int; COMMIT; \
                     CREATE INDEX CONCURRENTLY i ON users (x);";
    assert!(rules(&run(committed, TransactionMode::None)).is_empty());
    let open =
        "BEGIN; ALTER TABLE users ADD COLUMN x int; CREATE INDEX CONCURRENTLY i ON users (x);";
    assert_eq!(
        rules(&run(open, TransactionMode::None)),
        ["cannot-run-in-transaction"]
    );
}

#[test]
fn rollback_also_ends_transaction() {
    let sql = "ROLLBACK; VACUUM users;";
    assert!(rules(&run(sql, TransactionMode::PerUnit)).is_empty());
}

#[test]
fn concurrently_on_new_table_still_fails_in_transaction() {
    let unit = run(
        "CREATE TABLE t (c int); CREATE INDEX CONCURRENTLY i ON t (c);",
        TransactionMode::PerUnit,
    );
    assert_eq!(rules(&unit), ["cannot-run-in-transaction"]);
}

#[test]
fn volatile_default_on_existing_table_is_an_error() {
    let sql = "ALTER TABLE users ADD COLUMN token uuid DEFAULT gen_random_uuid();";
    let unit = run(sql, TransactionMode::PerUnit);
    assert_eq!(rules(&unit), ["add-column-rewrite"]);
    assert!(unit.statements[0].findings[0].message.contains("\"users\""));
    let new_table = format!("CREATE TABLE users (id int); {sql}");
    assert!(rules(&run(&new_table, TransactionMode::PerUnit)).is_empty());
}

#[test]
fn safe_add_column_has_no_findings() {
    let unit = run(
        "ALTER TABLE users ADD COLUMN nick text;",
        TransactionMode::PerUnit,
    );
    assert!(rules(&unit).is_empty());
    assert_eq!(unit.statements.len(), 1);
}

#[test]
fn parse_error_is_a_unit_finding() {
    let unit = run("CREATE INDEX ON;", TransactionMode::PerUnit);
    assert!(unit.statements.is_empty());
    assert_eq!(unit.findings[0].rule, "parse-error");
    assert_eq!(unit.findings[0].severity, Severity::Error);
    assert!(
        unit.findings[0]
            .message
            .starts_with("Could not parse this file (line 1): ")
    );
    assert_eq!(unit.findings[0].line, Some(1));
}

#[test]
fn counts_and_max_severity() {
    let units = [
        MigrationUnit {
            name: "a.sql".into(),
            sql: "CREATE INDEX i ON users (email);".into(),
        },
        MigrationUnit {
            name: "b.sql".into(),
            sql: "SELECT 1; SELECT 2;".into(),
        },
    ];
    let report = analyze(&units, &Config::default());
    assert_eq!(report.units[0].name, "a.sql");
    assert_eq!(
        report.counts(),
        Counts {
            errors: 1,
            warnings: 0,
            suppressed: 0,
            files: 2,
            statements: 3,
            not_analyzed: 2
        }
    );
    assert_eq!(report.max_severity(), Some(Severity::Error));

    let clean = analyze(&units[1..], &Config::default());
    assert_eq!(clean.max_severity(), None);
}

#[test]
fn create_table_if_not_exists_is_not_a_new_table() {
    let unit = run(
        "CREATE TABLE IF NOT EXISTS users (id int); CREATE INDEX i ON users (id);",
        TransactionMode::PerUnit,
    );
    assert_eq!(rules(&unit), ["create-index-not-concurrently"]);
    assert!(!unit.statements[1].new_table);
}

#[test]
fn tables_created_from_queries_are_new() {
    for sql in [
        "CREATE TABLE t AS SELECT * FROM users; CREATE INDEX i ON t (id);",
        "SELECT * INTO t FROM users; CREATE INDEX i ON t (id);",
        "CREATE MATERIALIZED VIEW t AS SELECT * FROM users; CREATE INDEX i ON t (id);",
    ] {
        let unit = run(sql, TransactionMode::PerUnit);
        assert!(rules(&unit).is_empty(), "{sql}: {:?}", rules(&unit));
        assert!(unit.statements[1].new_table, "{sql}");
    }
    let maybe_existing = run(
        "CREATE TABLE IF NOT EXISTS t AS SELECT * FROM users; CREATE INDEX i ON t (id);",
        TransactionMode::PerUnit,
    );
    assert_eq!(rules(&maybe_existing), ["create-index-not-concurrently"]);
}

#[test]
fn counts_not_analyzed() {
    let units = [MigrationUnit {
        name: "m.sql".into(),
        sql: "SELECT 1; ALTER TABLE users ADD COLUMN a int;".into(),
    }];
    let counts = analyze(&units, &Config::default()).counts();
    assert_eq!(counts.not_analyzed, 1);
    assert_eq!(counts.statements, 2);
}

#[test]
fn effect_new_table_flag() {
    let unit = run(
        "CREATE TABLE users (id int); CREATE INDEX i ON users (id);",
        TransactionMode::PerUnit,
    );
    let Analysis::Locks(effects) = &unit.statements[1].analysis else {
        panic!("expected locks");
    };
    assert!(effects[0].new_table);
}

#[test]
fn per_effect_new_table() {
    let unit = run(
        "CREATE TABLE new_orders (id int, user_id int); \
         ALTER TABLE new_orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id);",
        TransactionMode::PerUnit,
    );
    let stmt = &unit.statements[1];
    let Analysis::Locks(effects) = &stmt.analysis else {
        panic!("expected locks");
    };
    assert!(effects[0].new_table);
    assert!(!effects[1].new_table);
    assert!(!stmt.new_table);
    assert!(rules(&unit).is_empty());
}
