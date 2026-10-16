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

// --- Task 9: columns ---

#[test]
fn add_column_not_null_without_default_fires() {
    let sql = "ALTER TABLE users ADD COLUMN a int NOT NULL;";
    assert_eq!(fired(sql, PerUnit), ["add-column-not-null-without-default"]);
    let f = finding(sql, PerUnit, "add-column-not-null-without-default");
    assert_eq!(
        f.message,
        "Adding a NOT NULL column without a DEFAULT fails if \"users\" has any rows."
    );
}

#[test]
fn add_column_not_null_without_default_skips() {
    for sql in [
        "ALTER TABLE users ADD COLUMN a int NOT NULL DEFAULT 0;",
        "ALTER TABLE users ADD COLUMN a int GENERATED ALWAYS AS IDENTITY NOT NULL;",
        "ALTER TABLE users ADD COLUMN a bigserial NOT NULL;",
        "ALTER TABLE users ADD COLUMN a int GENERATED ALWAYS AS (id * 2) STORED NOT NULL;",
        "CREATE TABLE t (id int); ALTER TABLE t ADD COLUMN a int NOT NULL;",
    ] {
        assert!(
            !fired(sql, PerUnit).contains(&"add-column-not-null-without-default"),
            "{sql}"
        );
    }
}

#[test]
fn add_column_unknown_default_fires() {
    let sql = "ALTER TABLE users ADD COLUMN code text DEFAULT make_code();";
    assert_eq!(fired(sql, PerUnit), ["add-column-unknown-default"]);
    let f = finding(sql, PerUnit, "add-column-unknown-default");
    assert_eq!(
        f.message,
        "lockline cannot tell whether the default make_code() is volatile; if it is, \"users\" is rewritten."
    );
    let with_args = finding(
        "ALTER TABLE users ADD COLUMN code text DEFAULT make_code(8);",
        PerUnit,
        "add-column-unknown-default",
    );
    assert!(with_args.message.contains("make_code(...)"));
}

#[test]
fn add_column_unknown_default_skips() {
    for sql in [
        "ALTER TABLE users ADD COLUMN a text DEFAULT 'x';",
        "ALTER TABLE users ADD COLUMN a timestamptz DEFAULT now();",
        "CREATE TABLE t (id int); ALTER TABLE t ADD COLUMN a text DEFAULT make_code();",
    ] {
        assert!(fired(sql, PerUnit).is_empty(), "{sql}");
    }
    // Known volatile: add-column-rewrite reports it, not this rule.
    assert_eq!(
        fired(
            "ALTER TABLE users ADD COLUMN a uuid DEFAULT gen_random_uuid();",
            PerUnit
        ),
        ["add-column-rewrite"]
    );
}

#[test]
fn alter_column_type_fires() {
    let sql = "ALTER TABLE users ALTER COLUMN email TYPE varchar(300);";
    assert_eq!(fired(sql, PerUnit), ["alter-column-type"]);
    let f = finding(sql, PerUnit, "alter-column-type");
    assert_eq!(
        f.message,
        "Changing the type of \"email\" rewrites \"users\" and its indexes under an ACCESS EXCLUSIVE lock."
    );
    let help = f.help.unwrap();
    assert!(help.starts_with("If this only widens varchar(n) or numeric precision"));
    assert!(help.contains("Otherwise add a new column"));

    let known = "CREATE TABLE users (email varchar(300)); \
                 ALTER TABLE users ADD COLUMN x int; \
                 SELECT 1;";
    let narrowing = format!("{known} ALTER TABLE users ALTER COLUMN email TYPE varchar(100);");
    // users was created in this unit, so nothing fires; check the help text through a
    // second unit instead.
    assert!(fired(&narrowing, PerUnit).is_empty());
}

#[test]
fn alter_column_type_knows_the_previous_type_across_units() {
    let units = [
        MigrationUnit {
            name: "0001.sql".into(),
            sql: "CREATE TABLE users (email varchar(100));".into(),
        },
        MigrationUnit {
            name: "0002.sql".into(),
            sql: "ALTER TABLE users ALTER COLUMN email TYPE varchar(200);".into(),
        },
        MigrationUnit {
            name: "0003.sql".into(),
            sql: "ALTER TABLE users ALTER COLUMN email TYPE varchar(50);".into(),
        },
    ];
    let report = analyze(&units, &Config::default());
    let rules_of = |i: usize| -> Vec<&str> {
        report.units[i]
            .statements
            .iter()
            .flat_map(|s| &s.findings)
            .map(|f| f.rule)
            .filter(|r| *r != "missing-lock-timeout")
            .collect()
    };
    assert!(rules_of(1).is_empty(), "widening is safe");
    assert_eq!(rules_of(2), ["alter-column-type"]);
    let help = report.units[2].statements[0].findings[0]
        .help
        .clone()
        .unwrap();
    assert_eq!(
        help,
        "Add a new column, backfill it in batches, and switch reads and writes over."
    );
}

#[test]
fn set_not_null_fires() {
    let sql = "ALTER TABLE users ALTER COLUMN email SET NOT NULL;";
    assert_eq!(fired(sql, PerUnit), ["set-not-null"]);
    let help = finding(sql, PerUnit, "set-not-null").help.unwrap();
    assert_eq!(
        help,
        "ALTER TABLE users ADD CONSTRAINT users_email_not_null CHECK (email IS NOT NULL) NOT VALID;\n\
         ALTER TABLE users VALIDATE CONSTRAINT users_email_not_null;\n\
         ALTER TABLE users ALTER COLUMN email SET NOT NULL;\n\
         ALTER TABLE users DROP CONSTRAINT users_email_not_null;"
    );
}

#[test]
fn set_not_null_skips() {
    let units = [
        MigrationUnit {
            name: "0001.sql".into(),
            sql: "CREATE TABLE users (email text);".into(),
        },
        MigrationUnit {
            name: "0002.sql".into(),
            sql: "ALTER TABLE users ADD CONSTRAINT c CHECK (email IS NOT NULL) NOT VALID;".into(),
        },
        MigrationUnit {
            name: "0003.sql".into(),
            sql: "ALTER TABLE users VALIDATE CONSTRAINT c; \
                  ALTER TABLE users ALTER COLUMN email SET NOT NULL;"
                .into(),
        },
    ];
    let report = analyze(&units, &Config::default());
    assert!(
        report.units[2]
            .statements
            .iter()
            .flat_map(|s| &s.findings)
            .all(|f| f.rule != "set-not-null")
    );
    assert!(
        fired(
            "CREATE TABLE t (a int); ALTER TABLE t ALTER COLUMN a SET NOT NULL;",
            PerUnit
        )
        .is_empty()
    );
}

#[test]
fn add_column_rewrite_ignores_other_subcommands() {
    assert_eq!(
        fired("ALTER TABLE users ALTER COLUMN a TYPE bigint;", PerUnit),
        ["alter-column-type"]
    );
}

// --- Task 10: constraints ---

#[test]
fn add_check_without_not_valid_fires() {
    let sql = "ALTER TABLE users ADD CONSTRAINT positive_age CHECK (age > 0);";
    assert_eq!(fired(sql, PerUnit), ["add-check-without-not-valid"]);
    let f = finding(sql, PerUnit, "add-check-without-not-valid");
    assert_eq!(
        f.message,
        "Adding a CHECK constraint scans all of \"users\" under an ACCESS EXCLUSIVE lock."
    );
    assert_eq!(
        f.help.unwrap(),
        "ALTER TABLE users ADD CONSTRAINT positive_age CHECK (age > 0) NOT VALID;\n\
         ALTER TABLE users VALIDATE CONSTRAINT positive_age;"
    );
    let unnamed = finding(
        "ALTER TABLE users ADD CHECK (age > 0);",
        PerUnit,
        "add-check-without-not-valid",
    );
    assert!(
        unnamed
            .help
            .unwrap()
            .contains("VALIDATE CONSTRAINT users_check;")
    );
    let inline = finding(
        "ALTER TABLE users ADD COLUMN age int CHECK (age > 0);",
        PerUnit,
        "add-check-without-not-valid",
    );
    let help = inline.help.unwrap();
    assert!(
        help.starts_with("Add the column without the constraint, then:\n"),
        "{help}"
    );
    assert!(help.contains("CHECK (age > 0) NOT VALID;"), "{help}");
}

#[test]
fn add_check_without_not_valid_skips() {
    assert!(
        fired(
            "ALTER TABLE users ADD CONSTRAINT c CHECK (age > 0) NOT VALID;",
            PerUnit
        )
        .is_empty()
    );
    assert!(
        fired(
            "CREATE TABLE t (age int); ALTER TABLE t ADD CHECK (age > 0);",
            PerUnit
        )
        .is_empty()
    );
}

#[test]
fn add_foreign_key_without_not_valid_fires() {
    let sql = "ALTER TABLE orders ADD CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES users (id);";
    assert_eq!(fired(sql, PerUnit), ["add-foreign-key-without-not-valid"]);
    let f = finding(sql, PerUnit, "add-foreign-key-without-not-valid");
    assert_eq!(
        f.message,
        "Adding a foreign key scans \"orders\" and blocks writes to \"orders\" and \"users\" until it is validated."
    );
    let help = f.help.unwrap();
    assert!(help.contains("REFERENCES users (id) NOT VALID;"), "{help}");
    assert!(
        help.ends_with("ALTER TABLE orders VALIDATE CONSTRAINT orders_user_fk;"),
        "{help}"
    );

    let inline = finding(
        "ALTER TABLE orders ADD COLUMN user_id bigint REFERENCES users (id);",
        PerUnit,
        "add-foreign-key-without-not-valid",
    );
    let help = inline.help.unwrap();
    assert!(
        help.contains("CONSTRAINT orders_user_id_fkey FOREIGN KEY (user_id)"),
        "{help}"
    );
}

#[test]
fn add_foreign_key_without_not_valid_skips() {
    assert!(fired("ALTER TABLE orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id) NOT VALID;", PerUnit).is_empty());
    let new_source = "CREATE TABLE orders (id int, user_id int); \
                      ALTER TABLE orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id);";
    assert!(fired(new_source, PerUnit).is_empty());
}

#[test]
fn add_unique_without_index_fires() {
    let sql = "ALTER TABLE users ADD CONSTRAINT users_email_key UNIQUE (email);";
    assert_eq!(fired(sql, PerUnit), ["add-unique-without-index"]);
    let f = finding(sql, PerUnit, "add-unique-without-index");
    assert_eq!(
        f.message,
        "Adding a UNIQUE constraint builds an index under an ACCESS EXCLUSIVE lock on \"users\"."
    );
    assert_eq!(
        f.help.unwrap(),
        "CREATE UNIQUE INDEX CONCURRENTLY users_email_key_idx ON users (email);\n\
         ALTER TABLE users ADD CONSTRAINT users_email_key UNIQUE USING INDEX users_email_key_idx;"
    );
    let pk = finding(
        "ALTER TABLE users ADD PRIMARY KEY (id);",
        PerUnit,
        "add-unique-without-index",
    );
    assert!(pk.message.starts_with("Adding a PRIMARY KEY constraint"));
    assert!(
        pk.help
            .unwrap()
            .contains("ADD CONSTRAINT users_pkey PRIMARY KEY USING INDEX users_pkey_idx;")
    );
    let inline = finding(
        "ALTER TABLE users ADD COLUMN code text UNIQUE;",
        PerUnit,
        "add-unique-without-index",
    );
    assert!(inline.help.unwrap().contains("ON users (code);"));
}

#[test]
fn add_unique_without_index_skips() {
    assert!(
        fired(
            "ALTER TABLE users ADD CONSTRAINT u UNIQUE USING INDEX users_email_idx;",
            PerUnit
        )
        .is_empty()
    );
    assert!(
        fired(
            "CREATE TABLE t (id int); ALTER TABLE t ADD PRIMARY KEY (id);",
            PerUnit
        )
        .is_empty()
    );
}

// --- Task 11: compatibility and lock_timeout ---

#[test]
fn rename_object_fires() {
    let f = finding(
        "ALTER TABLE users RENAME TO accounts;",
        PerUnit,
        "rename-object",
    );
    assert_eq!(
        f.message,
        "Renaming \"users\" breaks application code that still uses the old name while the deploy rolls out."
    );
    let f = finding(
        "ALTER TABLE users RENAME COLUMN email TO mail;",
        PerUnit,
        "rename-object",
    );
    assert!(f.message.starts_with("Renaming \"email\""));
}

#[test]
fn rename_object_skips() {
    assert!(fired("ALTER TABLE users RENAME CONSTRAINT a TO b;", PerUnit).is_empty());
    assert!(
        fired(
            "CREATE TABLE t (a int); ALTER TABLE t RENAME COLUMN a TO b;",
            PerUnit
        )
        .is_empty()
    );
}

#[test]
fn drop_object_fires() {
    let f = finding("DROP TABLE users;", PerUnit, "drop-object");
    assert_eq!(
        f.message,
        "Dropping \"users\" breaks application code that still uses it while the deploy rolls out."
    );
    let f = finding(
        "ALTER TABLE users DROP COLUMN legacy;",
        PerUnit,
        "drop-object",
    );
    assert!(f.message.starts_with("Dropping \"legacy\""));
}

#[test]
fn drop_object_skips() {
    assert!(fired("CREATE TABLE t (a int); DROP TABLE t;", PerUnit).is_empty());
    assert!(
        fired(
            "CREATE TABLE t (a int); ALTER TABLE t DROP COLUMN a;",
            PerUnit
        )
        .is_empty()
    );
}

#[test]
fn missing_lock_timeout_fires() {
    let sql = "ALTER TABLE users ADD COLUMN a int;";
    assert_eq!(fired_all(sql, PerUnit), ["missing-lock-timeout"]);
    let f = finding(sql, PerUnit, "missing-lock-timeout");
    assert_eq!(f.severity, lockline::model::Severity::Warning);
    assert_eq!(
        f.message,
        "This migration takes an ACCESS EXCLUSIVE lock on \"users\" without a lock_timeout; if the lock has to wait, every query on the table queues behind it."
    );
}

#[test]
fn missing_lock_timeout_skips() {
    assert!(
        fired_all(
            "SET lock_timeout = '5s'; ALTER TABLE users ADD COLUMN a int;",
            PerUnit
        )
        .is_empty()
    );
    assert!(
        fired_all(
            "SET LOCAL lock_timeout = '5s'; ALTER TABLE users ADD COLUMN a int;",
            PerUnit
        )
        .is_empty()
    );
    assert!(
        fired_all(
            "CREATE TABLE t (a int); ALTER TABLE t ADD COLUMN b int;",
            PerUnit
        )
        .is_empty()
    );
    // SHARE is not ACCESS EXCLUSIVE.
    assert!(!fired_all("CREATE INDEX i ON users (a);", PerUnit).contains(&"missing-lock-timeout"));
}

#[test]
fn missing_lock_timeout_once_per_unit() {
    let sql = "ALTER TABLE users ADD COLUMN a int; \
               ALTER TABLE users ADD COLUMN b int; \
               ALTER TABLE orders ADD COLUMN c int;";
    assert_eq!(fired_all(sql, PerUnit), ["missing-lock-timeout"]);
}
