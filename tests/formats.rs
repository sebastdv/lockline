use lockline::input::parse_pretend;
use lockline::report::{render_github, render_sarif};
use lockline::{Config, MigrationUnit, analyze};

fn report_for(name: &str, sql: &str) -> lockline::Report {
    analyze(&[MigrationUnit::new(name, sql)], &Config::default())
}

#[test]
fn github_annotations_point_at_the_statement_and_escape_text() {
    let report = report_for(
        "./db/0001,add:index.sql",
        "SET lock_timeout = '5s';\n\nCREATE INDEX i ON users (email);",
    );
    let out = render_github(&report);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(
        lines[0].starts_with(
            "::error file=db/0001%2Cadd%3Aindex.sql,line=3,col=1,title=lockline%3A create-index-not-concurrently::Building an index"
        ),
        "{}",
        lines[0]
    );
    // Multi-line help is kept on one line with %0A, as GitHub expects.
    assert!(
        lines[0].contains("%0Ahelp: CREATE INDEX CONCURRENTLY i ON users USING btree (email);%0A")
    );
    assert_eq!(lines[1], "lockline: 1 error, 0 warnings in 1 file");
}

#[test]
fn github_annotations_for_laravel_land_on_the_php_file_and_quote_the_statement() {
    let output =
        "  2026_01_01_000000_add_index ......\n  ⇂ create index \"i\" on \"users\" (\"email\")\n";
    let report = analyze(
        &parse_pretend(output, "database/migrations"),
        &Config::default(),
    );
    let out = render_github(&report);
    let index = out
        .lines()
        .find(|l| l.contains("create-index-not-concurrently"))
        .unwrap();
    assert!(
        index.starts_with(
            "::error file=database/migrations/2026_01_01_000000_add_index.php,line=1,title="
        ),
        "{index}"
    );
    assert!(index.contains("%0AStatement: create index \"i\" on \"users\" (\"email\")"));
}

#[test]
fn stdin_annotations_have_no_file() {
    let out = render_github(&report_for("<stdin>", "CREATE INDEX i ON t (c);"));
    assert!(
        out.starts_with("::error title=lockline%3A create-index-not-concurrently::"),
        "{out}"
    );
}

#[test]
fn sarif_has_rules_results_locations_and_suppressions() {
    let sql = "-- lockline:ignore create-index-not-concurrently the table has 10 rows\n\
               CREATE INDEX small ON flags (name);\n\
               SET lock_timeout = '5s';\n\
               CREATE INDEX big ON users (email);";
    let sarif: serde_json::Value =
        serde_json::from_str(&render_sarif(&report_for("m/0001.sql", sql))).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    let run = &sarif["runs"][0];
    let rules = run["tool"]["driver"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), lockline::rules::infos().len());
    let index_rule = rules
        .iter()
        .position(|r| r["id"] == "create-index-not-concurrently")
        .unwrap();
    assert_eq!(rules[index_rule]["name"], "CreateIndexNotConcurrently");
    assert_eq!(
        rules[index_rule]["helpUri"],
        "https://github.com/sebastdv/lockline/blob/main/docs/rules/create-index-not-concurrently.md"
    );
    assert_eq!(rules[index_rule]["defaultConfiguration"]["level"], "error");

    let results = run["results"].as_array().unwrap();
    let index_results: Vec<&serde_json::Value> = results
        .iter()
        .filter(|r| r["ruleId"] == "create-index-not-concurrently")
        .collect();
    assert_eq!(index_results.len(), 2);
    let suppressed = index_results
        .iter()
        .find(|r| r.get("suppressions").is_some())
        .unwrap();
    assert_eq!(
        suppressed["suppressions"][0]["justification"],
        "the table has 10 rows"
    );
    assert_eq!(
        suppressed["locations"][0]["physicalLocation"]["region"]["startLine"],
        2
    );
    let reported = index_results
        .iter()
        .find(|r| r.get("suppressions").is_none())
        .unwrap();
    assert_eq!(reported["level"], "error");
    assert_eq!(reported["ruleIndex"], index_rule);
    assert_eq!(
        reported["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "m/0001.sql"
    );
    assert_eq!(
        reported["locations"][0]["physicalLocation"]["region"]["startLine"],
        4
    );
}

#[test]
fn sarif_from_stdin_has_no_locations() {
    let sarif: serde_json::Value = serde_json::from_str(&render_sarif(&report_for(
        "<stdin>",
        "CREATE INDEX i ON t (c);",
    )))
    .unwrap();
    let result = &sarif["runs"][0]["results"][0];
    assert!(result.get("locations").is_none());
}
