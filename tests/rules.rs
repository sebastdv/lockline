use lockline::TransactionMode::{self, None as Autocommit, PerUnit};
use lockline::model::Finding;
use lockline::{Config, MigrationUnit, UnitReport, analyze};

// `Config` gains fields in later tasks; the struct update keeps this helper stable.
#[allow(clippy::needless_update)]
fn unit(sql: &str, mode: TransactionMode) -> UnitReport {
    let units = [MigrationUnit {
        name: "m.sql".into(),
        sql: sql.into(),
    }];
    let config = Config {
        transaction: mode,
        ..Config::default()
    };
    analyze(&units, &config).units.remove(0)
}

fn findings(sql: &str, mode: TransactionMode) -> Vec<Finding> {
    let unit = unit(sql, mode);
    unit.findings
        .into_iter()
        .chain(unit.statements.into_iter().flat_map(|s| s.findings))
        .collect()
}

/// Every rule that fired, including missing-lock-timeout.
fn fired_all(sql: &str, mode: TransactionMode) -> Vec<&'static str> {
    findings(sql, mode).into_iter().map(|f| f.rule).collect()
}

/// Like `fired_all`, without missing-lock-timeout (it has its own tests).
fn fired(sql: &str, mode: TransactionMode) -> Vec<&'static str> {
    fired_all(sql, mode)
        .into_iter()
        .filter(|r| *r != "missing-lock-timeout")
        .collect()
}

fn finding(sql: &str, mode: TransactionMode, rule: &str) -> Finding {
    findings(sql, mode)
        .into_iter()
        .find(|f| f.rule == rule)
        .unwrap_or_else(|| panic!("{rule} did not fire for {sql:?}"))
}

// --- Task 8: indexes and maintenance ---

#[test]
fn drop_index_not_concurrently_fires() {
    assert_eq!(
        fired("DROP INDEX i;", Autocommit),
        ["drop-index-not-concurrently"]
    );
    let f = finding("DROP INDEX i;", Autocommit, "drop-index-not-concurrently");
    assert_eq!(f.severity, lockline::model::Severity::Warning);
    assert!(f.help.unwrap().contains("DROP INDEX CONCURRENTLY i"));
}

#[test]
fn drop_index_not_concurrently_skips() {
    assert!(fired("DROP INDEX CONCURRENTLY i;", Autocommit).is_empty());
    let new_table = "CREATE TABLE t (x int); CREATE INDEX i ON t (x); DROP INDEX i;";
    assert!(fired(new_table, PerUnit).is_empty());
}

#[test]
fn reindex_not_concurrently_fires() {
    for sql in ["REINDEX TABLE users;", "REINDEX INDEX users_pkey;"] {
        assert_eq!(
            fired(sql, Autocommit),
            ["reindex-not-concurrently"],
            "{sql}"
        );
    }
    let f = finding(
        "REINDEX TABLE users;",
        Autocommit,
        "reindex-not-concurrently",
    );
    assert_eq!(
        f.message,
        "REINDEX without CONCURRENTLY blocks writes to \"users\" while the indexes are rebuilt."
    );
    assert!(f.help.unwrap().contains("CONCURRENTLY"));
}

#[test]
fn reindex_not_concurrently_skips() {
    assert!(fired("REINDEX TABLE CONCURRENTLY users;", Autocommit).is_empty());
    assert!(fired("REINDEX (CONCURRENTLY) TABLE users;", Autocommit).is_empty());
    assert!(fired("REINDEX DATABASE app;", Autocommit).is_empty());
    assert!(fired("CREATE TABLE t (x int); REINDEX TABLE t;", PerUnit).is_empty());
}

#[test]
fn rewrite_table_fires() {
    let f = finding("VACUUM FULL users;", Autocommit, "rewrite-table");
    assert!(f.message.starts_with("VACUUM FULL rewrites \"users\""));
    let f = finding(
        "CLUSTER users USING users_pkey;",
        Autocommit,
        "rewrite-table",
    );
    assert!(f.message.starts_with("CLUSTER rewrites \"users\""));
    assert!(f.help.unwrap().contains("pg_repack"));
}

#[test]
fn rewrite_table_skips() {
    assert!(fired("VACUUM users;", Autocommit).is_empty());
    assert!(fired("CREATE TABLE t (x int); CLUSTER t USING t_pkey;", PerUnit).is_empty());
}

#[test]
fn refresh_matview_not_concurrently_fires() {
    let f = finding(
        "REFRESH MATERIALIZED VIEW stats;",
        PerUnit,
        "refresh-matview-not-concurrently",
    );
    assert!(
        f.help
            .unwrap()
            .starts_with("REFRESH MATERIALIZED VIEW CONCURRENTLY stats;")
    );
}

#[test]
fn refresh_matview_not_concurrently_skips() {
    assert!(fired("REFRESH MATERIALIZED VIEW CONCURRENTLY stats;", PerUnit).is_empty());
    let created =
        "CREATE MATERIALIZED VIEW stats AS SELECT 1 AS n; REFRESH MATERIALIZED VIEW stats;";
    assert!(fired(created, PerUnit).is_empty());
}

#[test]
fn backfill_in_migration_fires() {
    assert_eq!(
        fired("UPDATE users SET a = 1;", PerUnit),
        ["backfill-in-migration"]
    );
    let f = finding("DELETE FROM users;", PerUnit, "backfill-in-migration");
    assert!(f.message.starts_with("Deleting every row of \"users\""));
}

#[test]
fn backfill_in_migration_skips() {
    assert!(fired("UPDATE users SET a = 1 WHERE id < 100;", PerUnit).is_empty());
    assert!(fired("DELETE FROM users WHERE id = 1;", PerUnit).is_empty());
    assert!(fired("CREATE TABLE t (a int); UPDATE t SET a = 1;", PerUnit).is_empty());
}
