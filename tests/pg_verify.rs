//! Verifies the lock catalog against a real PostgreSQL.
//!
//! Every case runs in its own schema. lockline's catalog says which locks the statement
//! takes and whether it rewrites the table; this test runs the statement and compares with
//! `pg_locks` and `pg_class.relfilenode`. When they disagree, PostgreSQL wins.
//!
//! Needs `LOCKLINE_TEST_DATABASE_URL` (see docker-compose.yml); without it the tests are
//! skipped, unless `LOCKLINE_REQUIRE_PG=1`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use lockline::catalog::analyze_statement;
use lockline::locks::LockMode;
use lockline::model::{Analysis, Cost};
use lockline::parse::parse_sql;
use lockline::schema::Schema;
use postgres::{Client, NoTls};

/// Tables every case starts from (inside its own schema).
const BASE_SETUP: &str = r#"
CREATE TABLE orgs (id bigint PRIMARY KEY);
CREATE TABLE users (id bigint PRIMARY KEY, email text, name varchar(100), age int, org_id bigint);
CREATE INDEX users_email_idx ON users (email);
CREATE TABLE orders (id bigint PRIMARY KEY, user_id bigint, created_at timestamptz);
CREATE TABLE events (id bigint, at date) PARTITION BY RANGE (at);
CREATE TABLE events_2025 PARTITION OF events FOR VALUES FROM ('2025-01-01') TO ('2026-01-01');
CREATE TABLE events_2026 (id bigint, at date);
CREATE MATERIALIZED VIEW stats AS SELECT 1 AS n;
CREATE UNIQUE INDEX stats_n_idx ON stats (n);
CREATE VIEW v AS SELECT 1 AS n;
CREATE TABLE plain (id int);
CREATE FUNCTION touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE TRIGGER users_touch BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION touch();
INSERT INTO orgs SELECT g FROM generate_series(1, 3) g;
INSERT INTO users SELECT g, 'u' || g, 'n', 20, 1 FROM generate_series(1, 10) g;
INSERT INTO plain SELECT g FROM generate_series(1, 10) g;
"#;

const BLOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// Table kinds whose locks the catalog describes: tables, partitioned tables, materialized
/// views, views and foreign tables. Index, sequence and TOAST locks are not modeled.
const TABLE_KINDS: [&str; 5] = ["r", "p", "m", "v", "f"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    /// BEGIN; statement; read our own locks; ROLLBACK.
    InTransaction,
    /// For statements that cannot run in a transaction: another session holds the table
    /// and we read the lock the statement waits for.
    Blocking,
}

#[derive(Debug, Clone, Copy)]
struct Case {
    extra_setup: &'static str,
    sql: &'static str,
    run: Run,
    /// Table the blocking session holds; `None` means the catalog's `effects[0]`.
    block_on: Option<&'static str>,
}

const fn tx(sql: &'static str) -> Case {
    tx_after("", sql)
}

const fn tx_after(extra_setup: &'static str, sql: &'static str) -> Case {
    Case {
        extra_setup,
        sql,
        run: Run::InTransaction,
        block_on: None,
    }
}

const fn blocking(sql: &'static str) -> Case {
    Case {
        extra_setup: "",
        sql,
        run: Run::Blocking,
        block_on: None,
    }
}

const fn blocking_on(table: &'static str, sql: &'static str) -> Case {
    Case {
        extra_setup: "",
        sql,
        run: Run::Blocking,
        block_on: Some(table),
    }
}

fn resolve_url(url: Option<String>, require: bool) -> Option<String> {
    match url {
        Some(url) => Some(url),
        None if require => {
            panic!("LOCKLINE_TEST_DATABASE_URL is not set but LOCKLINE_REQUIRE_PG=1")
        }
        None => {
            println!("skipped: LOCKLINE_TEST_DATABASE_URL not set");
            None
        }
    }
}

fn database_url() -> Option<String> {
    resolve_url(
        std::env::var("LOCKLINE_TEST_DATABASE_URL")
            .ok()
            .filter(|u| !u.is_empty()),
        std::env::var("LOCKLINE_REQUIRE_PG").is_ok_and(|v| v == "1"),
    )
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Observed {
    /// Strongest lock per pre-existing table-like relation, stronger than ACCESS SHARE.
    locks: BTreeMap<String, LockMode>,
    /// Pre-existing relations whose relfilenode changed (the table was rewritten).
    rewritten: BTreeSet<String>,
}

/// oid -> (name, relfilenode, relkind) for the relations of the case's schema.
type Relations = BTreeMap<u32, (String, u32, String)>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn connect(url: &str, schema: &str) -> Result<Client, String> {
    let mut client = Client::connect(url, NoTls).map_err(err)?;
    client
        .batch_execute(&format!("SET search_path TO {schema}"))
        .map_err(err)?;
    Ok(client)
}

fn relations(client: &mut Client, schema: &str) -> Result<Relations, String> {
    let rows = client
        .query(
            &format!(
                "SELECT oid, relname::text, relfilenode, relkind::text FROM pg_class \
                 WHERE relnamespace = '{schema}'::regnamespace"
            ),
            &[],
        )
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<_, u32>(0), (r.get(1), r.get(2), r.get(3))))
        .collect())
}

/// Locks held by this session on relations, by oid (works for relations dropped or
/// renamed inside the transaction, which pg_class no longer shows by their old name).
fn own_locks(client: &mut Client) -> Result<Vec<(u32, String)>, String> {
    let rows = client
        .query(
            "SELECT relation, mode FROM pg_locks \
             WHERE pid = pg_backend_pid() AND locktype = 'relation' \
               AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
            &[],
        )
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

fn lock_mode(pg: &str) -> Option<LockMode> {
    Some(match pg {
        "AccessShareLock" => LockMode::AccessShare,
        "RowShareLock" => LockMode::RowShare,
        "RowExclusiveLock" => LockMode::RowExclusive,
        "ShareUpdateExclusiveLock" => LockMode::ShareUpdateExclusive,
        "ShareLock" => LockMode::Share,
        "ShareRowExclusiveLock" => LockMode::ShareRowExclusive,
        "ExclusiveLock" => LockMode::Exclusive,
        "AccessExclusiveLock" => LockMode::AccessExclusive,
        _ => return None,
    })
}

fn build(before: &Relations, after: &Relations, modes: &[(u32, String)]) -> Observed {
    let mut observed = Observed::default();
    for (oid, mode) in modes {
        let Some((name, _, kind)) = before.get(oid) else {
            continue;
        };
        let Some(mode) = lock_mode(mode) else {
            continue;
        };
        if !TABLE_KINDS.contains(&kind.as_str()) || mode <= LockMode::AccessShare {
            continue;
        }
        let entry = observed.locks.entry(name.clone()).or_insert(mode);
        *entry = (*entry).max(mode);
    }
    for (oid, (name, filenode, kind)) in before {
        let changed = after
            .get(oid)
            .is_some_and(|(_, new_filenode, _)| new_filenode != filenode);
        if changed && TABLE_KINDS.contains(&kind.as_str()) {
            observed.rewritten.insert(name.clone());
        }
    }
    observed
}

/// What lockline's catalog says about the case, with the setup seen by the virtual schema.
fn expected(case: &Case) -> Analysis {
    let mut schema = Schema::default();
    for setup in [BASE_SETUP, case.extra_setup] {
        for stmt in parse_sql(setup).expect("setup parses") {
            schema.observe(&stmt.node);
        }
    }
    let mut stmts = parse_sql(case.sql).expect("case parses");
    assert_eq!(stmts.len(), 1, "one statement per case: {}", case.sql);
    analyze_statement(&stmts.remove(0).node, &schema)
}

fn observe(url: &str, case: &Case, schema: &str) -> Result<Observed, String> {
    let mut admin = Client::connect(url, NoTls).map_err(err)?;
    admin
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}"
        ))
        .map_err(err)?;
    let result = observe_in_schema(url, case, schema);
    let _ = admin.batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"));
    result
}

fn observe_in_schema(url: &str, case: &Case, schema: &str) -> Result<Observed, String> {
    let mut a = connect(url, schema)?;
    a.batch_execute(BASE_SETUP)
        .map_err(|e| format!("setup: {e}"))?;
    if !case.extra_setup.is_empty() {
        a.batch_execute(case.extra_setup)
            .map_err(|e| format!("extra setup: {e}"))?;
    }
    let before = relations(&mut a, schema)?;
    match case.run {
        Run::InTransaction => {
            a.batch_execute("BEGIN").map_err(err)?;
            a.batch_execute(case.sql)
                .map_err(|e| format!("statement failed: {e}"))?;
            let modes = own_locks(&mut a)?;
            let after = relations(&mut a, schema)?;
            a.batch_execute("ROLLBACK").map_err(err)?;
            Ok(build(&before, &after, &modes))
        }
        Run::Blocking => observe_blocked(url, case, schema, a, &before),
    }
}

fn observe_blocked(
    url: &str,
    case: &Case,
    schema: &str,
    mut a: Client,
    before: &Relations,
) -> Result<Observed, String> {
    let target = match case.block_on {
        Some(table) => table.to_string(),
        None => match expected(case) {
            Analysis::Locks(effects) => effects[0].table.name.clone(),
            other => return Err(format!("no table to block on: {other:?}")),
        },
    };
    let target_oid = before
        .iter()
        .find(|(_, (name, _, _))| *name == target)
        .map(|(oid, _)| *oid)
        .ok_or_else(|| format!("{target} is not in the setup"))?;
    let mut holder = connect(url, schema)?;
    holder
        // SHARE UPDATE EXCLUSIVE lets brief ACCESS SHARE locks through (VACUUM takes one to
        // expand its relation list) and still stops every lock the catalog claims for these
        // statements, so the wait we observe is the real one.
        .batch_execute(&format!(
            "BEGIN; LOCK TABLE {target} IN SHARE UPDATE EXCLUSIVE MODE"
        ))
        .map_err(err)?;
    let pid: i32 = a
        .query_one("SELECT pg_backend_pid()", &[])
        .map_err(err)?
        .get(0);
    let sql = case.sql.to_string();
    let runner = std::thread::spawn(move || {
        let result = a.batch_execute(&sql).map_err(err);
        (a, result)
    });
    let mut watcher = connect(url, schema)?;
    let deadline = Instant::now() + BLOCK_TIMEOUT;
    let mode = loop {
        let rows = watcher
            .query(
                "SELECT mode FROM pg_locks WHERE pid = $1 AND NOT granted AND relation = $2",
                &[&pid, &target_oid],
            )
            .map_err(err)?;
        if let Some(row) = rows.first() {
            break Some(row.get::<_, String>(0));
        }
        if Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    holder.batch_execute("ROLLBACK").map_err(err)?;
    let (mut a, result) = runner
        .join()
        .map_err(|_| "statement thread panicked".to_string())?;
    let Some(mode) = mode else {
        return Err(format!(
            "timed out waiting for {:?} to block on {target}",
            case.sql
        ));
    };
    result.map_err(|e| format!("statement failed: {e}"))?;
    let after = relations(&mut a, schema)?;
    Ok(build(before, &after, &[(target_oid, mode)]))
}

/// Human-readable disagreements between the catalog and PostgreSQL.
fn compare(case: &Case, expected: &Analysis, observed: &Observed) -> Vec<String> {
    let sql = case.sql;
    let mut problems = Vec::new();
    match expected {
        Analysis::Locks(effects) => {
            for effect in effects {
                let table = &effect.table.name;
                match observed.locks.get(table) {
                    Some(lock) if *lock == effect.lock => {}
                    Some(lock) => problems.push(format!(
                        "{sql}: catalog says {} on {table}, PostgreSQL took {}",
                        effect.lock.as_str(),
                        lock.as_str()
                    )),
                    None => problems.push(format!(
                        "{sql}: catalog says {} on {table}, PostgreSQL took nothing stronger than ACCESS SHARE",
                        effect.lock.as_str()
                    )),
                }
                let rewritten = observed.rewritten.contains(table);
                match effect.cost {
                    Cost::Rewrite if !rewritten => problems.push(format!(
                        "{sql}: catalog says {table} is rewritten, it was not"
                    )),
                    // TRUNCATE swaps in an empty file: a new relfilenode, but no rewrite
                    // whose cost grows with the table.
                    Cost::Metadata | Cost::Scan if rewritten && !is_truncate(sql) => problems.push(
                        format!("{sql}: catalog says {table} is not rewritten, it was"),
                    ),
                    _ => {}
                }
            }
            for (table, lock) in &observed.locks {
                if !effects.iter().any(|e| &e.table.name == table) {
                    problems.push(format!(
                        "{sql}: extra lock {} on {table} not in the catalog",
                        lock.as_str()
                    ));
                }
            }
        }
        Analysis::NoTableLock => {
            for (table, lock) in &observed.locks {
                problems.push(format!(
                    "{sql}: catalog says no table lock, PostgreSQL took {} on {table}",
                    lock.as_str()
                ));
            }
        }
        Analysis::NotAnalyzed => problems.push(format!(
            "{sql}: the catalog does not analyze this statement"
        )),
    }
    problems
}

fn is_truncate(sql: &str) -> bool {
    sql.trim_start()
        .to_ascii_uppercase()
        .starts_with("TRUNCATE")
}

static SCHEMA_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn unique_schema() -> String {
    format!(
        "lv_{}_{}",
        std::process::id(),
        SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Runs every case and fails once with the full list of disagreements.
fn verify(cases: &[Case]) {
    let Some(url) = database_url() else {
        return;
    };
    let mut problems = Vec::new();
    for case in cases {
        let expected = expected(case);
        match observe(&url, case, &unique_schema()) {
            Ok(observed) => problems.extend(compare(case, &expected, &observed)),
            Err(e) => problems.push(format!("{}: {e}", case.sql)),
        }
    }
    assert!(
        problems.is_empty(),
        "{} disagreement(s) with PostgreSQL:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

// --- Task 1: harness ---

#[test]
fn harness_cases() {
    verify(&[
        tx("CREATE INDEX i ON users (email)"),
        tx("ALTER TABLE users ADD COLUMN token uuid DEFAULT gen_random_uuid()"),
        blocking("CREATE INDEX CONCURRENTLY i2 ON users (name)"),
    ]);
}

#[test]
fn extra_locks_ignore_indexes_and_reads() {
    verify(&[tx("INSERT INTO users VALUES (99, 'x', 'n', 1, 1)")]);
}

#[test]
#[should_panic(expected = "LOCKLINE_REQUIRE_PG=1")]
fn require_pg_fails_without_url() {
    resolve_url(None, true);
}

#[test]
fn resolve_url_cases() {
    assert_eq!(resolve_url(None, false), None);
    assert_eq!(
        resolve_url(Some("postgres://x".into()), true),
        Some("postgres://x".to_string())
    );
}

#[test]
fn blocking_times_out_cleanly() {
    let Some(url) = database_url() else {
        return;
    };
    let started = std::time::Instant::now();
    let result = observe(&url, &blocking_on("users", "SELECT 1"), "lv_timeout");
    let err = result.expect_err("SELECT 1 never blocks");
    assert!(err.contains("timed out"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(12));
    // The lock was released: the next case is not stuck behind it.
    verify(&[tx("CREATE INDEX i ON users (email)")]);
}

// --- Task 2: ALTER TABLE ---

#[test]
fn alter_table_matches_postgres() {
    verify(&[
        tx("ALTER TABLE users ADD COLUMN nick text"),
        tx("ALTER TABLE users ADD COLUMN created timestamptz DEFAULT now()"),
        tx("ALTER TABLE users ADD COLUMN n bigserial"),
        tx("ALTER TABLE users ADD COLUMN ident int GENERATED ALWAYS AS IDENTITY"),
        tx("ALTER TABLE users ADD COLUMN twice int GENERATED ALWAYS AS (age * 2) STORED"),
        tx("ALTER TABLE users ADD COLUMN code text UNIQUE"),
        tx("ALTER TABLE users ADD COLUMN c2 int CHECK (c2 > 0)"),
        tx("ALTER TABLE users ADD COLUMN org2 bigint REFERENCES orgs (id)"),
        tx("ALTER TABLE users DROP COLUMN name"),
        tx("ALTER TABLE users ALTER COLUMN age SET DEFAULT 1"),
        tx("ALTER TABLE users ALTER COLUMN age DROP DEFAULT"),
        tx("ALTER TABLE users ALTER COLUMN age DROP NOT NULL"),
        tx("ALTER TABLE users ALTER COLUMN age TYPE bigint"),
        tx("ALTER TABLE users ALTER COLUMN name TYPE varchar(200)"),
        tx("ALTER TABLE users ALTER COLUMN name TYPE varchar(50)"),
        tx("ALTER TABLE users ALTER COLUMN age SET NOT NULL"),
        tx_after(
            "ALTER TABLE users ADD CONSTRAINT age_nn CHECK (age IS NOT NULL);",
            "ALTER TABLE users ALTER COLUMN age SET NOT NULL",
        ),
        tx("ALTER TABLE users ADD CONSTRAINT c CHECK (age > 0)"),
        tx("ALTER TABLE users ADD CONSTRAINT c CHECK (age > 0) NOT VALID"),
        tx("ALTER TABLE users ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id)"),
        tx(
            "ALTER TABLE users ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id) NOT VALID",
        ),
        tx("ALTER TABLE users ADD CONSTRAINT u UNIQUE (email)"),
        tx_after(
            "CREATE UNIQUE INDEX users_email_uidx ON users (email);",
            "ALTER TABLE users ADD CONSTRAINT u UNIQUE USING INDEX users_email_uidx",
        ),
        tx_after(
            "CREATE TABLE logs (id bigint NOT NULL); CREATE UNIQUE INDEX logs_id_idx ON logs (id);",
            "ALTER TABLE logs ADD CONSTRAINT logs_pkey PRIMARY KEY USING INDEX logs_id_idx",
        ),
        tx_after(
            "ALTER TABLE users ADD CONSTRAINT age_pos CHECK (age > 0) NOT VALID;",
            "ALTER TABLE users VALIDATE CONSTRAINT age_pos",
        ),
        tx_after(
            "ALTER TABLE users ADD CONSTRAINT users_org_fk FOREIGN KEY (org_id) REFERENCES orgs (id) NOT VALID;",
            "ALTER TABLE users VALIDATE CONSTRAINT users_org_fk",
        ),
        tx("ALTER TABLE users ALTER COLUMN age SET STATISTICS 100"),
        tx("ALTER TABLE users ALTER COLUMN age SET (n_distinct = 10)"),
        tx("ALTER TABLE users ALTER COLUMN age RESET (n_distinct)"),
        tx("ALTER TABLE users SET (fillfactor = 70)"),
        tx("ALTER TABLE users RESET (fillfactor)"),
        tx("ALTER TABLE users CLUSTER ON users_pkey"),
        tx("ALTER TABLE users SET WITHOUT CLUSTER"),
        tx_after(
            "CREATE TABLE idents (id int GENERATED ALWAYS AS IDENTITY);",
            "ALTER TABLE idents ALTER COLUMN id SET GENERATED BY DEFAULT",
        ),
        tx_after(
            "CREATE TABLE idents (id int GENERATED ALWAYS AS IDENTITY);",
            "ALTER TABLE idents ALTER COLUMN id DROP IDENTITY",
        ),
        tx_after(
            "ALTER TABLE plain ALTER COLUMN id SET NOT NULL;",
            "ALTER TABLE plain ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY",
        ),
        tx("ALTER TABLE users REPLICA IDENTITY FULL"),
        tx("ALTER TABLE users OWNER TO CURRENT_USER"),
        tx("ALTER TABLE users ENABLE ROW LEVEL SECURITY"),
        tx("ALTER TABLE users FORCE ROW LEVEL SECURITY"),
        tx_after(
            "ALTER TABLE orders ADD CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES users (id);",
            "ALTER TABLE orders ALTER CONSTRAINT orders_user_fk DEFERRABLE",
        ),
        tx("ALTER TABLE users ENABLE TRIGGER ALL"),
        tx("ALTER TABLE users DISABLE TRIGGER users_touch"),
        tx("ALTER TABLE plain SET UNLOGGED"),
        tx_after(
            "ALTER TABLE plain SET UNLOGGED;",
            "ALTER TABLE plain SET LOGGED",
        ),
        tx(
            "ALTER TABLE events ATTACH PARTITION events_2026 FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')",
        ),
        tx("ALTER TABLE events DETACH PARTITION events_2025"),
        tx("ALTER TABLE users RENAME TO people"),
        tx("ALTER TABLE users RENAME COLUMN email TO mail"),
        tx("ALTER TABLE users RENAME CONSTRAINT users_pkey TO users_pk"),
    ]);
}

// --- Task 3: other statements ---

#[test]
fn statements_match_postgres() {
    verify(&[
        tx("CREATE INDEX i ON users (email)"),
        tx("DROP TABLE orders"),
        tx("DROP INDEX users_email_idx"),
        tx("TRUNCATE orders"),
        tx("REINDEX TABLE users"),
        tx("REINDEX INDEX users_email_idx"),
        tx("ANALYZE users"),
        tx("CLUSTER users USING users_pkey"),
        tx("REFRESH MATERIALIZED VIEW stats"),
        tx("REFRESH MATERIALIZED VIEW CONCURRENTLY stats"),
        tx("CREATE TRIGGER t2 AFTER UPDATE ON users FOR EACH ROW EXECUTE FUNCTION touch()"),
        tx("INSERT INTO users VALUES (50, 'x', 'n', 1, 1)"),
        tx("UPDATE users SET age = 2"),
        tx("DELETE FROM users WHERE id = 1"),
        tx(
            "CREATE TABLE events_2027 PARTITION OF events FOR VALUES FROM ('2027-01-01') TO ('2028-01-01')",
        ),
        tx("CREATE TABLE kids (extra int) INHERITS (plain)"),
        tx("CREATE TABLE memberships (org_id bigint REFERENCES orgs (id))"),
        tx("CREATE TABLE fresh (id int)"),
        tx("CREATE OR REPLACE VIEW v AS SELECT 1 AS n"),
        tx("COMMENT ON TABLE users IS 'x'"),
        tx("COMMENT ON COLUMN users.email IS 'x'"),
    ]);
}

#[test]
fn non_transactional_match_postgres() {
    verify(&[
        blocking("CREATE INDEX CONCURRENTLY i3 ON users (age)"),
        blocking("DROP INDEX CONCURRENTLY users_email_idx"),
        blocking("REINDEX TABLE CONCURRENTLY users"),
        blocking("ALTER TABLE events DETACH PARTITION events_2025 CONCURRENTLY"),
        blocking("VACUUM users"),
        blocking("VACUUM FULL users"),
    ]);
}
