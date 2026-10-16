use lockline::catalog::{analyze_statement, forbidden_in_transaction};
use lockline::locks::LockMode;
use lockline::model::{Analysis, Cost, Effect, TableRef};
use lockline::parse::parse_sql;

fn node(sql: &str) -> pg_query::NodeEnum {
    let mut stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1, "expected one statement in {sql:?}");
    stmts.remove(0).node
}

fn analyze(sql: &str) -> Analysis {
    analyze_statement(&node(sql))
}

fn single_effect(sql: &str) -> Effect {
    match analyze(sql) {
        Analysis::Locks(mut effects) => {
            assert_eq!(effects.len(), 1, "expected one effect for {sql:?}");
            effects.remove(0)
        }
        other => panic!("expected locks for {sql:?}, got {other:?}"),
    }
}

fn users() -> TableRef {
    TableRef {
        schema: None,
        name: "users".into(),
    }
}

#[test]
fn create_index_takes_share_and_scans() {
    let e = single_effect("CREATE INDEX i ON users (email)");
    assert_eq!(e.table, users());
    assert_eq!(e.lock, LockMode::Share);
    assert_eq!(e.cost, Cost::Scan);
}

#[test]
fn create_index_concurrently_takes_share_update_exclusive() {
    let e = single_effect("CREATE INDEX CONCURRENTLY i ON users (email)");
    assert_eq!(e.lock, LockMode::ShareUpdateExclusive);
    assert_eq!(e.cost, Cost::Scan);
}

#[test]
fn add_column_without_default_is_metadata() {
    let e = single_effect("ALTER TABLE users ADD COLUMN nick text");
    assert_eq!(e.table, users());
    assert_eq!(e.lock, LockMode::AccessExclusive);
    assert_eq!(e.cost, Cost::Metadata);
}

#[test]
fn add_column_with_constant_default_is_metadata() {
    let e = single_effect("ALTER TABLE users ADD COLUMN active boolean DEFAULT true");
    assert_eq!(e.cost, Cost::Metadata);
}

#[test]
fn add_column_with_now_default_is_metadata() {
    for sql in [
        "ALTER TABLE users ADD COLUMN created_at timestamptz DEFAULT now()",
        "ALTER TABLE users ADD COLUMN created_at timestamptz DEFAULT CURRENT_TIMESTAMP",
        "ALTER TABLE users ADD COLUMN status text DEFAULT 'new'::text",
    ] {
        assert_eq!(single_effect(sql).cost, Cost::Metadata, "{sql}");
    }
}

#[test]
fn add_column_with_volatile_default_rewrites() {
    let e = single_effect("ALTER TABLE users ADD COLUMN token uuid DEFAULT gen_random_uuid()");
    assert_eq!(e.lock, LockMode::AccessExclusive);
    assert_eq!(e.cost, Cost::Rewrite);
}

#[test]
fn add_serial_or_identity_column_rewrites() {
    for sql in [
        "ALTER TABLE users ADD COLUMN n bigserial",
        "ALTER TABLE users ADD COLUMN n int GENERATED ALWAYS AS IDENTITY",
    ] {
        assert_eq!(single_effect(sql).cost, Cost::Rewrite, "{sql}");
    }
}

#[test]
fn add_stored_generated_column_rewrites() {
    let e = single_effect("ALTER TABLE users ADD COLUMN t int GENERATED ALWAYS AS (a + 1) STORED");
    assert_eq!(e.cost, Cost::Rewrite);
}

#[test]
fn several_add_columns_take_the_highest_cost() {
    let e = single_effect(
        "ALTER TABLE users ADD COLUMN a int, ADD COLUMN b uuid DEFAULT gen_random_uuid()",
    );
    assert_eq!(e.cost, Cost::Rewrite);
}

#[test]
fn schema_qualified_table_is_kept() {
    let e = single_effect("CREATE INDEX i ON app.users (x)");
    assert_eq!(
        e.table,
        TableRef {
            schema: Some("app".into()),
            name: "users".into()
        }
    );
}

#[test]
fn create_table_has_no_table_lock() {
    assert_eq!(analyze("CREATE TABLE t (id int)"), Analysis::NoTableLock);
}

#[test]
fn set_has_no_table_lock() {
    assert_eq!(analyze("SET lock_timeout = '2s'"), Analysis::NoTableLock);
}

#[test]
fn begin_has_no_table_lock() {
    assert_eq!(analyze("BEGIN"), Analysis::NoTableLock);
}

#[test]
fn drop_column_is_not_analyzed_yet() {
    assert_eq!(
        analyze("ALTER TABLE users DROP COLUMN x"),
        Analysis::NotAnalyzed
    );
}

#[test]
fn mixed_alter_table_is_not_analyzed_yet() {
    assert_eq!(
        analyze("ALTER TABLE users ADD COLUMN a int, DROP COLUMN b"),
        Analysis::NotAnalyzed
    );
}

#[test]
fn forbidden_in_transaction_labels() {
    let cases = [
        (
            "CREATE INDEX CONCURRENTLY i ON t (c)",
            Some("CREATE INDEX CONCURRENTLY"),
        ),
        ("DROP INDEX CONCURRENTLY i", Some("DROP INDEX CONCURRENTLY")),
        ("REINDEX INDEX CONCURRENTLY i", Some("REINDEX CONCURRENTLY")),
        (
            "ALTER TABLE p DETACH PARTITION c CONCURRENTLY",
            Some("DETACH PARTITION CONCURRENTLY"),
        ),
        ("VACUUM t", Some("VACUUM")),
        ("VACUUM", Some("VACUUM")),
        ("CREATE INDEX i ON t (c)", None),
        ("DROP INDEX i", None),
        ("ANALYZE t", None),
        ("SELECT 1", None),
    ];
    for (sql, expected) in cases {
        assert_eq!(forbidden_in_transaction(&node(sql)), expected, "{sql}");
    }
}

#[test]
fn create_table_that_locks_other_tables_is_not_analyzed_yet() {
    for sql in [
        "CREATE TABLE events_2026_10 PARTITION OF events FOR VALUES FROM ('2026-10-01') TO ('2026-11-01')",
        "CREATE TABLE orders (id int, user_id int REFERENCES users (id))",
        "CREATE TABLE orders (id int, user_id int, FOREIGN KEY (user_id) REFERENCES users (id))",
        "CREATE TABLE child (extra int) INHERITS (parent)",
        "CREATE OR REPLACE VIEW v AS SELECT 1",
    ] {
        assert_eq!(analyze(sql), Analysis::NotAnalyzed, "{sql}");
    }
    assert_eq!(analyze("CREATE VIEW v AS SELECT 1"), Analysis::NoTableLock);
}

#[test]
fn add_column_with_inline_constraints_is_not_analyzed_yet() {
    for sql in [
        "ALTER TABLE posts ADD COLUMN user_id bigint REFERENCES users (id)",
        "ALTER TABLE posts ADD COLUMN code text UNIQUE",
        "ALTER TABLE posts ADD COLUMN id2 int PRIMARY KEY",
        "ALTER TABLE posts ADD COLUMN status text DEFAULT 'a' CHECK (status <> '')",
    ] {
        assert_eq!(analyze(sql), Analysis::NotAnalyzed, "{sql}");
    }
    let e = single_effect("ALTER TABLE posts ADD COLUMN title text NOT NULL DEFAULT ''");
    assert_eq!(e.cost, Cost::Metadata);
}
