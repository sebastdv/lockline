use lockline::report::{HumanOptions, render_human};
use lockline::suppress::{Ignore, parse_ignores};
use lockline::{Config, MigrationUnit, Report, analyze};

fn report(sql: &str) -> Report {
    let units = [MigrationUnit {
        name: "m.sql".into(),
        sql: sql.into(),
    }];
    analyze(&units, &Config::default())
}

fn rules(report: &Report) -> Vec<&'static str> {
    let unit = &report.units[0];
    unit.findings
        .iter()
        .chain(unit.statements.iter().flat_map(|s| s.findings.iter()))
        .map(|f| f.rule)
        .filter(|r| *r != "missing-lock-timeout")
        .collect()
}

#[test]
fn parse_ignores_forms() {
    let sql = "-- lockline:ignore create-index-not-concurrently table is tiny\n\
               SELECT 1;\n\
               --lockline:ignore a,b\n\
               -- lockline:ignore-file drop-object we own every caller\n\
               SELECT 2; -- lockline:ignore trailing comments do not count\n";
    assert_eq!(
        parse_ignores(sql),
        [
            Ignore {
                line: 1,
                rules: vec!["create-index-not-concurrently".into()],
                reason: Some("table is tiny".into()),
                file_wide: false,
            },
            Ignore {
                line: 3,
                rules: vec!["a".into(), "b".into()],
                reason: None,
                file_wide: false,
            },
            Ignore {
                line: 4,
                rules: vec!["drop-object".into()],
                reason: Some("we own every caller".into()),
                file_wide: true,
            },
        ]
    );
}

#[test]
fn ignore_suppresses_next_statement() {
    let r = report(
        "-- lockline:ignore create-index-not-concurrently table is tiny\n\
         CREATE INDEX i ON users (e);",
    );
    assert!(rules(&r).is_empty());
    let suppressed = &r.units[0].statements[0].suppressed;
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0].rule, "create-index-not-concurrently");
    assert_eq!(suppressed[0].reason.as_deref(), Some("table is tiny"));
    assert_eq!(r.counts().suppressed, 1);
}

#[test]
fn ignore_does_not_leak_past_statement() {
    let r = report(
        "CREATE INDEX a ON t (x);\n\
         -- lockline:ignore create-index-not-concurrently ok\n\
         SELECT 1;\n\
         CREATE INDEX b ON t (y);",
    );
    assert_eq!(
        rules(&r),
        [
            "create-index-not-concurrently",
            "create-index-not-concurrently"
        ]
    );

    let trailing =
        report("CREATE INDEX c ON t (z); -- lockline:ignore create-index-not-concurrently ok");
    assert_eq!(rules(&trailing), ["create-index-not-concurrently"]);

    let inside = report(
        "SELECT\n\
         -- lockline:ignore create-index-not-concurrently ok\n\
         1;\n\
         CREATE INDEX d ON t (w);",
    );
    assert_eq!(rules(&inside), ["create-index-not-concurrently"]);
}

#[test]
fn ignore_file_applies_everywhere() {
    let r = report(
        "CREATE INDEX a ON t (x);\n\
         -- lockline:ignore-file create-index-not-concurrently tables are empty in this project\n\
         CREATE INDEX b ON t (y);",
    );
    assert!(rules(&r).is_empty());
    assert_eq!(r.counts().suppressed, 2);
}

#[test]
fn ignore_without_reason_warns() {
    let r = report(
        "-- lockline:ignore create-index-not-concurrently\n\
         CREATE INDEX i ON users (e);",
    );
    assert_eq!(rules(&r), ["ignore-without-reason"]);
    let f = &r.units[0].statements[0].findings[0];
    assert_eq!(
        f.message,
        "lockline:ignore for create-index-not-concurrently has no reason; say why this is safe."
    );
    assert_eq!(r.counts().suppressed, 1);

    let file_wide = report("-- lockline:ignore-file drop-object\nSELECT 1;");
    let f = &file_wide.units[0].findings[0];
    assert_eq!(f.rule, "ignore-without-reason");
    assert_eq!(f.line, Some(1));
}

#[test]
fn renders_suppressed_summary() {
    let r = report(
        "SET lock_timeout = '5s';\n\
         -- lockline:ignore create-index-not-concurrently table is tiny\n\
         CREATE INDEX i ON users (e);",
    );
    let out = render_human(
        &r,
        &HumanOptions {
            color: false,
            quiet: false,
        },
    );
    insta::assert_snapshot!(out);
}
