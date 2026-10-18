use lockline::report::{HumanOptions, render_human};
use lockline::{Config, MigrationUnit, Report, analyze};

fn report(units: &[(&str, &str)]) -> Report {
    let units: Vec<MigrationUnit> = units
        .iter()
        .map(|(name, sql)| MigrationUnit {
            source: Default::default(),
            name: name.to_string(),
            sql: sql.to_string(),
        })
        .collect();
    analyze(&units, &Config::default())
}

fn plain(report: &Report) -> String {
    render_human(
        report,
        &HumanOptions {
            color: false,
            quiet: false,
        },
    )
}

#[test]
fn renders_index_error_and_safe_column() {
    let r = report(&[(
        "migrations/0007_add_email_index.sql",
        "CREATE INDEX idx_users_email ON users (email);\nALTER TABLE users ADD COLUMN nickname text;\n",
    )]);
    insta::assert_snapshot!(plain(&r));
}

#[test]
fn renders_new_table_note_and_not_analyzed() {
    let r = report(&[(
        "0001_init.sql",
        "CREATE TABLE users (id bigint PRIMARY KEY, email text);\n\
         CREATE INDEX idx_users_email ON users (email);\n\
         GRANT SELECT ON users TO app;\n",
    )]);
    insta::assert_snapshot!(plain(&r));
}

#[test]
fn quiet_hides_clean_statements() {
    let r = report(&[
        (
            "clean.sql",
            "SET lock_timeout = '5s';\nALTER TABLE users ADD COLUMN nickname text;",
        ),
        (
            "dangerous.sql",
            "SET lock_timeout = '5s';\nALTER TABLE users ADD COLUMN bio text;\nCREATE INDEX i ON users (email);",
        ),
    ]);
    let out = render_human(
        &r,
        &HumanOptions {
            color: false,
            quiet: true,
        },
    );
    insta::assert_snapshot!(out);
}

#[test]
fn pluralization_in_summary() {
    let one = plain(&report(&[("a.sql", "SELECT 1;")]));
    assert!(
        one.ends_with("0 errors, 0 warnings in 1 file (1 statement, 1 not analyzed)\n"),
        "{one}"
    );
    let many = plain(&report(&[
        ("a.sql", "CREATE INDEX i ON users (email);"),
        ("b.sql", "SELECT 1; SELECT 2;"),
    ]));
    assert!(
        many.ends_with("1 error, 0 warnings in 2 files (3 statements, 2 not analyzed)\n"),
        "{many}"
    );
}

#[test]
fn long_statement_is_truncated() {
    let r = report(&[(
        "long.sql",
        "ALTER TABLE users\n    ADD COLUMN a_rather_long_column_name_for_testing text,\n    ADD COLUMN another_long_column_name text;",
    )]);
    insta::assert_snapshot!(plain(&r));
}

#[test]
fn parse_error_rendered_under_unit() {
    let r = report(&[("broken.sql", "CREATE INDEX ON;")]);
    insta::assert_snapshot!(plain(&r));
}

#[test]
fn color_only_when_asked() {
    let r = report(&[("a.sql", "CREATE INDEX i ON users (email);")]);
    assert!(!plain(&r).contains('\x1b'));
    let colored = render_human(
        &r,
        &HumanOptions {
            color: true,
            quiet: false,
        },
    );
    assert!(colored.contains('\x1b'));
}

#[test]
fn renders_held_locks() {
    let r = report(&[(
        "0003_two_steps.sql",
        "ALTER TABLE users ADD COLUMN a int;\nCREATE INDEX CONCURRENTLY i ON orders (x);\nCREATE INDEX j ON orders (y);\n",
    )]);
    insta::assert_snapshot!(plain(&r));
}
