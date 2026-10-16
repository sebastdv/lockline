use lockline::catalog::{analyze_statement, forbidden_in_transaction};
use lockline::locks::LockMode;
use lockline::model::{Analysis, Cost, Effect, TableRef};
use lockline::parse::parse_sql;
use lockline::schema::Schema;

fn node(sql: &str) -> pg_query::NodeEnum {
    let mut stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1, "expected one statement in {sql:?}");
    stmts.remove(0).node
}

fn analyze(sql: &str) -> Analysis {
    analyze_statement(&node(sql), &Schema::default())
}

/// Analyzes `sql` after `setup` has been observed by the virtual schema.
fn analyze_after(setup: &str, sql: &str) -> Analysis {
    let mut schema = Schema::default();
    for stmt in parse_sql(setup).unwrap() {
        schema.observe(&stmt.node);
    }
    analyze_statement(&node(sql), &schema)
}

fn effects(sql: &str) -> Vec<Effect> {
    match analyze(sql) {
        Analysis::Locks(effects) => effects,
        other => panic!("expected locks for {sql:?}, got {other:?}"),
    }
}

fn table(name: &str) -> TableRef {
    TableRef {
        schema: None,
        name: name.into(),
    }
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
fn alter_table_catalog() {
    use LockMode::*;
    let cases = [
        (
            "ALTER TABLE users DROP COLUMN a",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a DROP NOT NULL",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a SET DEFAULT 1",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a DROP DEFAULT",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users DROP CONSTRAINT c",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a TYPE bigint",
            AccessExclusive,
            Cost::Rewrite,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a SET NOT NULL",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT c CHECK (a > 0)",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT c CHECK (a > 0) NOT VALID",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id)",
            ShareRowExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id) NOT VALID",
            ShareRowExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT u UNIQUE (email)",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT u UNIQUE USING INDEX users_email_idx",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ADD PRIMARY KEY (id)",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD CONSTRAINT x EXCLUDE USING gist (r WITH &&)",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users VALIDATE CONSTRAINT c",
            ShareUpdateExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a SET STATISTICS 100",
            ShareUpdateExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a SET (n_distinct = 10)",
            ShareUpdateExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN a RESET (n_distinct)",
            ShareUpdateExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users SET (fillfactor = 70)",
            ShareUpdateExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users RESET (fillfactor)",
            ShareUpdateExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users DETACH PARTITION users_2026",
            AccessExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users DETACH PARTITION users_2026 CONCURRENTLY",
            ShareUpdateExclusive,
            Cost::Metadata,
        ),
        (
            "ALTER TABLE users ADD COLUMN code text UNIQUE",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD COLUMN id2 int PRIMARY KEY",
            AccessExclusive,
            Cost::Scan,
        ),
        (
            "ALTER TABLE users ADD COLUMN status text DEFAULT 'a' CHECK (status <> '')",
            AccessExclusive,
            Cost::Scan,
        ),
    ];
    for (sql, lock, cost) in cases {
        let e = &effects(sql)[0];
        assert_eq!(e.table, users(), "{sql}");
        assert_eq!((e.lock, e.cost), (lock, cost), "{sql}");
    }
}

#[test]
fn notes_explain_the_cost() {
    let unique = &effects("ALTER TABLE users ADD CONSTRAINT u UNIQUE (email)")[0];
    assert_eq!(unique.note.as_deref(), Some("builds an index"));
    let check = &effects("ALTER TABLE users ADD COLUMN s text CHECK (s <> '')")[0];
    assert_eq!(check.note.as_deref(), Some("validates every row"));
}

#[test]
fn foreign_key_locks_the_referenced_table() {
    let e =
        effects("ALTER TABLE users ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id)");
    assert_eq!(e.len(), 2);
    assert_eq!(e[1].table, table("orgs"));
    assert_eq!(
        (e[1].lock, e[1].cost),
        (LockMode::ShareRowExclusive, Cost::Metadata)
    );
}

#[test]
fn add_column_with_inline_references_has_two_effects() {
    let e = effects("ALTER TABLE posts ADD COLUMN user_id bigint REFERENCES users (id)");
    assert_eq!(e[0].table, table("posts"));
    assert_eq!(
        (e[0].lock, e[0].cost),
        (LockMode::AccessExclusive, Cost::Scan)
    );
    assert_eq!(e[0].note.as_deref(), Some("validates the foreign key"));
    assert_eq!(e[1].table, users());
    assert_eq!(
        (e[1].lock, e[1].cost),
        (LockMode::ShareRowExclusive, Cost::Metadata)
    );
}

#[test]
fn attach_partition_locks_parent_and_partition() {
    let e = effects(
        "ALTER TABLE events ATTACH PARTITION events_2026 FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')",
    );
    assert_eq!(e[0].table, table("events"));
    assert_eq!(
        (e[0].lock, e[0].cost),
        (LockMode::ShareUpdateExclusive, Cost::Metadata)
    );
    assert_eq!(e[1].table, table("events_2026"));
    assert_eq!(
        (e[1].lock, e[1].cost),
        (LockMode::AccessExclusive, Cost::Scan)
    );
    assert_eq!(
        e[1].note.as_deref(),
        Some("validates the partition constraint")
    );
}

#[test]
fn several_subcommands_take_the_strongest_lock_and_cost() {
    let e = effects(
        "ALTER TABLE users ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id) NOT VALID, \
         ADD CONSTRAINT fk2 FOREIGN KEY (team_id) REFERENCES orgs (id), DROP COLUMN legacy",
    );
    assert_eq!(
        (e[0].lock, e[0].cost),
        (LockMode::AccessExclusive, Cost::Scan)
    );
    assert_eq!(e.len(), 2, "orgs appears once: {e:?}");
}

#[test]
fn alter_column_type_widening_uses_schema() {
    let setup = "CREATE TABLE users (email varchar(100));";
    let cost = |sql: &str| match analyze_after(setup, sql) {
        Analysis::Locks(e) => e[0].cost,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        cost("ALTER TABLE users ALTER COLUMN email TYPE varchar(200)"),
        Cost::Metadata
    );
    assert_eq!(
        cost("ALTER TABLE users ALTER COLUMN email TYPE varchar(50)"),
        Cost::Rewrite
    );
    assert_eq!(
        cost("ALTER TABLE users ALTER COLUMN email TYPE varchar(200) USING email::varchar(200)"),
        Cost::Rewrite
    );
    assert_eq!(
        single_effect("ALTER TABLE users ALTER COLUMN email TYPE varchar(200)").cost,
        Cost::Rewrite
    );
}

#[test]
fn set_not_null_after_validated_check_is_metadata() {
    let setup = "CREATE TABLE users (email text); \
                 ALTER TABLE users ADD CONSTRAINT c CHECK (email IS NOT NULL) NOT VALID; \
                 ALTER TABLE users VALIDATE CONSTRAINT c;";
    let Analysis::Locks(e) =
        analyze_after(setup, "ALTER TABLE users ALTER COLUMN email SET NOT NULL")
    else {
        panic!("expected locks");
    };
    assert_eq!(e[0].cost, Cost::Metadata);
}

#[test]
fn non_table_alter_is_not_analyzed() {
    for sql in [
        "ALTER TYPE addr ADD ATTRIBUTE zip text",
        "ALTER FOREIGN TABLE ft ADD COLUMN c int",
        "ALTER INDEX i SET (fillfactor = 70)",
    ] {
        assert_eq!(analyze(sql), Analysis::NotAnalyzed, "{sql}");
    }
}

#[test]
fn unknown_subcommand_is_not_analyzed() {
    assert_eq!(
        analyze("ALTER TABLE users ALTER COLUMN a SET STORAGE EXTERNAL"),
        Analysis::NotAnalyzed
    );
    assert_eq!(
        analyze("ALTER TABLE users ADD COLUMN a int, ALTER COLUMN a SET STORAGE EXTERNAL"),
        Analysis::NotAnalyzed
    );
}

/// (table, lock, cost) of every effect, or `None` when not analyzed.
fn summary_of(analysis: Analysis) -> Option<Vec<(String, LockMode, Cost)>> {
    match analysis {
        Analysis::Locks(effects) => Some(
            effects
                .into_iter()
                .map(|e| (e.table.to_string(), e.lock, e.cost))
                .collect(),
        ),
        Analysis::NoTableLock => Some(Vec::new()),
        Analysis::NotAnalyzed => None,
    }
}

#[test]
fn statement_catalog() {
    use Cost::*;
    use LockMode::*;
    let e = |t: &str, l, c| (t.to_string(), l, c);
    let cases = vec![
        (
            "DROP TABLE users, orders",
            Some(vec![
                e("users", AccessExclusive, Metadata),
                e("orders", AccessExclusive, Metadata),
            ]),
        ),
        ("DROP VIEW v", None),
        ("REINDEX TABLE users", Some(vec![e("users", Share, Scan)])),
        (
            "REINDEX TABLE CONCURRENTLY users",
            Some(vec![e("users", ShareUpdateExclusive, Scan)]),
        ),
        ("REINDEX DATABASE app", None),
        (
            "VACUUM FULL users",
            Some(vec![e("users", AccessExclusive, Rewrite)]),
        ),
        (
            "VACUUM users, orders",
            Some(vec![
                e("users", ShareUpdateExclusive, Scan),
                e("orders", ShareUpdateExclusive, Scan),
            ]),
        ),
        (
            "ANALYZE users",
            Some(vec![e("users", ShareUpdateExclusive, Scan)]),
        ),
        ("VACUUM", None),
        (
            "CLUSTER users USING users_pkey",
            Some(vec![e("users", AccessExclusive, Rewrite)]),
        ),
        ("CLUSTER", None),
        (
            "REFRESH MATERIALIZED VIEW stats",
            Some(vec![e("stats", AccessExclusive, Rewrite)]),
        ),
        (
            "REFRESH MATERIALIZED VIEW CONCURRENTLY stats",
            Some(vec![e("stats", Exclusive, Rewrite)]),
        ),
        (
            "CREATE TRIGGER t BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION f()",
            Some(vec![e("users", ShareRowExclusive, Metadata)]),
        ),
        (
            "TRUNCATE users, orders",
            Some(vec![
                e("users", AccessExclusive, Metadata),
                e("orders", AccessExclusive, Metadata),
            ]),
        ),
        (
            "INSERT INTO users (a) VALUES (1)",
            Some(vec![e("users", RowExclusive, Unknown)]),
        ),
        (
            "UPDATE users SET a = 1",
            Some(vec![e("users", RowExclusive, Unknown)]),
        ),
        (
            "DELETE FROM users WHERE id = 1",
            Some(vec![e("users", RowExclusive, Unknown)]),
        ),
        (
            "CREATE TABLE events_2026 PARTITION OF events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')",
            Some(vec![e("events", AccessExclusive, Metadata)]),
        ),
        (
            "CREATE TABLE child (x int) INHERITS (parent)",
            Some(vec![e("parent", ShareUpdateExclusive, Metadata)]),
        ),
        (
            "CREATE TABLE orders (id int, user_id int REFERENCES users (id))",
            Some(vec![e("users", ShareRowExclusive, Metadata)]),
        ),
        (
            "CREATE TABLE orders (id int, user_id int, FOREIGN KEY (user_id) REFERENCES users (id))",
            Some(vec![e("users", ShareRowExclusive, Metadata)]),
        ),
        (
            "CREATE OR REPLACE VIEW v AS SELECT 1",
            Some(vec![e("v", AccessExclusive, Metadata)]),
        ),
        ("CREATE VIEW v AS SELECT 1", Some(vec![])),
        (
            "COMMENT ON TABLE users IS 'people'",
            Some(vec![e("users", ShareUpdateExclusive, Metadata)]),
        ),
        (
            "COMMENT ON COLUMN users.email IS 'contact'",
            Some(vec![e("users", ShareUpdateExclusive, Metadata)]),
        ),
        (
            "COMMENT ON COLUMN app.users.email IS 'contact'",
            Some(vec![e("app.users", ShareUpdateExclusive, Metadata)]),
        ),
        ("COMMENT ON FUNCTION f() IS 'x'", None),
        (
            "ALTER TABLE users RENAME TO accounts",
            Some(vec![e("users", AccessExclusive, Metadata)]),
        ),
        (
            "ALTER TABLE users RENAME COLUMN a TO b",
            Some(vec![e("users", AccessExclusive, Metadata)]),
        ),
        (
            "ALTER TABLE users RENAME CONSTRAINT c TO d",
            Some(vec![e("users", AccessExclusive, Metadata)]),
        ),
        ("ALTER INDEX i RENAME TO j", None),
    ];
    for (sql, expected) in cases {
        assert_eq!(summary_of(analyze(sql)), expected, "{sql}");
    }
}

#[test]
fn drop_index_resolves_table_from_schema() {
    let known = analyze_after("CREATE INDEX i ON users (x);", "DROP INDEX i");
    assert_eq!(
        summary_of(known),
        Some(vec![(
            "users".to_string(),
            LockMode::AccessExclusive,
            Cost::Metadata
        )])
    );
    let unknown = effects("DROP INDEX i");
    assert_eq!(unknown[0].table, table("i"));
    assert_eq!(
        unknown[0].note.as_deref(),
        Some("on the index's table (not seen in this run)")
    );
    let concurrent = effects("DROP INDEX CONCURRENTLY i");
    assert_eq!(concurrent[0].lock, LockMode::ShareUpdateExclusive);
}

#[test]
fn reindex_index_notes() {
    let known = match analyze_after("CREATE INDEX i ON users (x);", "REINDEX INDEX i") {
        Analysis::Locks(e) => e,
        other => panic!("{other:?}"),
    };
    assert_eq!(known[0].table, users());
    assert_eq!(
        (known[0].lock, known[0].cost),
        (LockMode::Share, Cost::Scan)
    );
    assert_eq!(
        known[0].note.as_deref(),
        Some("rebuilds the index; ACCESS EXCLUSIVE on the index blocks queries that use it")
    );
    let unknown = effects("REINDEX INDEX i");
    assert_eq!(
        unknown[0].note.as_deref(),
        Some(
            "rebuilds the index; ACCESS EXCLUSIVE on the index blocks queries that use it; \
             on the index's table (not seen in this run)"
        )
    );
}

#[test]
fn def_elem_bool_cases() {
    assert_eq!(
        forbidden_in_transaction(&node("REINDEX (CONCURRENTLY false) TABLE t")),
        None
    );
    for sql in [
        "REINDEX (CONCURRENTLY true) TABLE t",
        "REINDEX (CONCURRENTLY) TABLE t",
        "REINDEX (CONCURRENTLY on) TABLE t",
    ] {
        assert_eq!(
            forbidden_in_transaction(&node(sql)),
            Some("REINDEX CONCURRENTLY"),
            "{sql}"
        );
    }
    let e = single_effect("VACUUM (FULL false) users");
    assert_eq!(
        (e.lock, e.cost),
        (LockMode::ShareUpdateExclusive, Cost::Scan)
    );
    let e = single_effect("VACUUM (FULL) users");
    assert_eq!((e.lock, e.cost), (LockMode::AccessExclusive, Cost::Rewrite));
}

#[test]
fn more_alter_table_subcommands() {
    use Cost::*;
    use LockMode::*;
    let cases = [
        (
            "ALTER TABLE users ENABLE TRIGGER ALL",
            ShareRowExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users DISABLE TRIGGER t",
            ShareRowExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users CLUSTER ON users_pkey",
            ShareUpdateExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users SET WITHOUT CLUSTER",
            ShareUpdateExclusive,
            Metadata,
        ),
        ("ALTER TABLE users SET LOGGED", AccessExclusive, Rewrite),
        ("ALTER TABLE users SET UNLOGGED", AccessExclusive, Rewrite),
        (
            "ALTER TABLE users REPLICA IDENTITY FULL",
            AccessExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users ENABLE ROW LEVEL SECURITY",
            AccessExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users FORCE ROW LEVEL SECURITY",
            AccessExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY",
            AccessExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN id DROP IDENTITY IF EXISTS",
            AccessExclusive,
            Metadata,
        ),
        (
            "ALTER TABLE users ALTER COLUMN id SET GENERATED BY DEFAULT",
            AccessExclusive,
            Metadata,
        ),
        ("ALTER TABLE users OWNER TO app", AccessExclusive, Metadata),
        (
            "ALTER TABLE users ALTER CONSTRAINT c DEFERRABLE",
            AccessExclusive,
            Metadata,
        ),
    ];
    for (sql, lock, cost) in cases {
        let e = &effects(sql)[0];
        assert_eq!((e.lock, e.cost), (lock, cost), "{sql}");
    }
}

#[test]
fn primary_key_using_index_may_scan() {
    let e = single_effect(
        "ALTER TABLE users ADD CONSTRAINT users_pkey PRIMARY KEY USING INDEX users_id_idx",
    );
    assert_eq!(e.cost, Cost::Scan);
    assert!(e.note.unwrap().contains("NOT NULL"));
}

#[test]
fn reindex_note_mentions_index_locks() {
    let e = single_effect("REINDEX TABLE users");
    assert!(e.note.unwrap().contains("ACCESS EXCLUSIVE on each index"));
}
