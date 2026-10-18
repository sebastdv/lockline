use std::path::PathBuf;

use lockline::input::{InputError, collect_laravel, parse_pretend};
use lockline::{Config, Source, analyze};

const PRETEND_13: &str = include_str!("fixtures/laravel/pretend-13.txt");

fn rules_fired(unit: &lockline::UnitReport) -> Vec<&'static str> {
    unit.all_findings().map(|f| f.rule).collect()
}

#[test]
fn reads_laravel_13_output_as_one_unit_per_migration() {
    let units = parse_pretend(PRETEND_13, "database/migrations");
    let names: Vec<&str> = units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "database/migrations/0001_01_01_000000_create_users_table.php",
            "database/migrations/0001_01_01_000001_create_cache_table.php",
            "database/migrations/0001_01_01_000002_create_jobs_table.php",
            "database/migrations/2026_10_17_000000_add_nickname_to_users.php",
        ]
    );
    let statements: Vec<usize> = units.iter().map(|u| u.sql.lines().count()).collect();
    assert_eq!(statements, [8, 6, 7, 4]);
    assert!(units.iter().all(|u| u.source == Source::Laravel));
    assert!(
        units[3]
            .sql
            .starts_with("alter table \"users\" add column \"nickname\" varchar(255) null;")
    );
}

#[test]
fn analyzes_laravel_output_with_context_across_migrations() {
    let report = analyze(
        &parse_pretend(PRETEND_13, "database/migrations"),
        &Config::default(),
    );
    // The first three migrations only touch tables they create.
    for unit in &report.units[..3] {
        assert_eq!(rules_fired(unit), Vec::<&str>::new(), "{}", unit.name);
    }
    // `->change()` repeats SET NOT NULL on a column created NOT NULL: no set-not-null finding.
    assert_eq!(
        rules_fired(&report.units[3]),
        [
            "missing-lock-timeout",
            "create-index-not-concurrently",
            "alter-column-type"
        ]
    );
}

#[test]
fn reads_multiline_statements_and_strips_colors() {
    let output = "\u{1b}[37;44m INFO \u{1b}[39;49m Running migrations.\n\n  \
        2026_01_01_000000_backfill \u{1b}[90m.........\u{1b}[39m\n  \
        ⇂ update users\n  set name = 'x'\n  where name is null  \n  \
        ⇂ create index concurrently i on users (name)\n";
    let units = parse_pretend(output, "db/migrations/");
    assert_eq!(units.len(), 1);
    assert_eq!(
        units[0].name,
        "db/migrations/2026_01_01_000000_backfill.php"
    );
    assert_eq!(
        units[0].sql,
        "update users\nset name = 'x'\nwhere name is null;\ncreate index concurrently i on users (name);"
    );
}

#[test]
fn reads_laravel_8_format() {
    let output = "CreateUsersTable: create table \"users\" (\"id\" bigserial primary key not null)\n\
        CreateUsersTable: alter table \"users\" add column \"email\" varchar(255) not null\n\
        AddIndexToUsers: create index \"users_email_index\" on \"users\" (\"email\")\n";
    let units = parse_pretend(output, "database/migrations");
    let names: Vec<&str> = units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["CreateUsersTable", "AddIndexToUsers"]);
    assert_eq!(units[0].sql.lines().count(), 2);
}

#[test]
fn explains_wrong_input() {
    let dir = PathBuf::from("tests/fixtures");
    let err = collect_laravel(&[dir], &mut std::io::empty(), "database/migrations").unwrap_err();
    assert!(matches!(err, InputError::Laravel { .. }));
    assert!(err.to_string().contains("is a directory"));

    let mut not_laravel: &[u8] = b"CREATE INDEX i ON t (c);";
    let err = collect_laravel(
        &[PathBuf::from("-")],
        &mut not_laravel,
        "database/migrations",
    )
    .unwrap_err();
    assert!(err.to_string().contains("no migrations found"));

    let mut nothing: &[u8] = b"\n   INFO  Nothing to migrate.\n";
    let collected =
        collect_laravel(&[PathBuf::from("-")], &mut nothing, "database/migrations").unwrap();
    assert!(collected.units.is_empty());
    assert_eq!(
        collected.warnings,
        ["<stdin>: Laravel had nothing to migrate"]
    );
}
