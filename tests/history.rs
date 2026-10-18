use std::fs;
use std::path::Path;

use lockline::input::{collect_history, collect_units};
use lockline::{Config, MigrationUnit, analyze, analyze_with_history};

fn write(dir: &Path, name: &str, sql: &str) {
    fs::write(dir.join(name), sql).unwrap();
}

#[test]
fn directory_history_uses_only_earlier_files_and_reports_nothing_about_them() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    write(
        m,
        "0001_create.sql",
        "CREATE TABLE t (id int PRIMARY KEY, n varchar(10) NOT NULL);",
    );
    write(
        m,
        "0002_widen.sql",
        "ALTER TABLE t ALTER COLUMN n TYPE varchar(20);",
    );
    write(m, "0003_later.sql", "DROP TABLE t;");

    let checked = collect_units(&[m.join("0002_widen.sql")], &mut std::io::empty())
        .unwrap()
        .units;
    let history = collect_history(m, &checked).unwrap().units;
    let names: Vec<String> = history
        .iter()
        .map(|u| {
            Path::new(&u.name)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["0001_create.sql"]);

    // Without history the old type is unknown, so the change is assumed to rewrite the table;
    // with it, varchar(10) → varchar(20) is a safe widening.
    let rules = |history: &[MigrationUnit]| -> Vec<&'static str> {
        let report = analyze_with_history(history, &checked, &Config::default());
        assert_eq!(report.units.len(), 1);
        report.units[0]
            .all_findings()
            .map(|f| f.rule)
            .filter(|r| *r != "missing-lock-timeout")
            .collect()
    };
    assert_eq!(rules(&[]), ["alter-column-type"]);
    assert_eq!(rules(&history), Vec::<&str>::new());
}

#[test]
fn schema_dump_history_skips_copy_data_and_psql_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let dump = tmp.path().join("pgsql-schema.sql");
    fs::write(
        &dump,
        "\\restrict abc123\nSET statement_timeout = 0;\n\
         CREATE TABLE public.users (id bigint NOT NULL, name character varying(255));\n\
         COPY public.migrations (id, migration, batch) FROM stdin;\n1\t0001_create_users\t1\n\\.\n\
         \\unrestrict abc123\n",
    )
    .unwrap();
    let checked = vec![MigrationUnit::new(
        "new.sql",
        "ALTER TABLE users ALTER COLUMN name TYPE varchar(300);",
    )];
    let history = collect_history(&dump, &checked).unwrap().units;
    let report = analyze_with_history(&history, &checked, &Config::default());
    let fired: Vec<&str> = report.units[0].all_findings().map(|f| f.rule).collect();
    assert!(!fired.contains(&"alter-column-type"), "{fired:?}");
    assert!(!fired.contains(&"parse-error"), "{fired:?}");
    // Sanity check: without the dump the type change is flagged.
    let alone = analyze(&checked, &Config::default());
    assert!(
        alone.units[0]
            .all_findings()
            .any(|f| f.rule == "alter-column-type")
    );
}
