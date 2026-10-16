use assert_cmd::Command;
use predicates::prelude::*;

fn lockline() -> Command {
    let mut cmd = Command::cargo_bin("lockline").unwrap();
    cmd.env_remove("NO_COLOR");
    cmd
}

#[test]
fn clean_directory_exits_zero() {
    lockline()
        .args(["check", "tests/fixtures/clean"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 errors, 0 warnings in 2 files"));
}

#[test]
fn dangerous_file_exits_one() {
    lockline()
        .args(["check", "tests/fixtures/dangerous/0001_index.sql"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "error[create-index-not-concurrently]",
        ));
}

#[test]
fn fail_on_warning_threshold() {
    lockline()
        .args(["check", "tests/fixtures/clean", "--fail-on", "warning"])
        .assert()
        .code(0);
    lockline()
        .args(["check", "tests/fixtures/dangerous", "--fail-on", "warning"])
        .assert()
        .code(1);
}

#[test]
fn stdin_input() {
    lockline()
        .args(["check", "-"])
        .write_stdin("CREATE INDEX i ON t (c);")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("<stdin>"));
}

#[test]
fn transaction_none_flag() {
    lockline()
        .args(["check", "-", "--transaction", "none"])
        .write_stdin("CREATE INDEX CONCURRENTLY i ON t (c);")
        .assert()
        .code(0);
    lockline()
        .args(["check", "-"])
        .write_stdin("CREATE INDEX CONCURRENTLY i ON t (c);")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("error[cannot-run-in-transaction]"));
}

#[test]
fn quiet_flag_hides_clean_statements() {
    lockline()
        .args(["check", "tests/fixtures/clean", "--quiet"])
        .assert()
        .code(0)
        .stdout("0 errors, 0 warnings in 2 files (4 statements)\n");
}

#[test]
fn missing_file_exits_two() {
    lockline()
        .args(["check", "nope/missing.sql"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("error: "))
        .stderr(predicate::str::contains("no such file or directory"));
}

#[test]
fn no_paths_exits_two() {
    lockline().arg("check").assert().code(2);
}

#[test]
fn unknown_flag_exits_two() {
    lockline()
        .args(["check", "x.sql", "--bogus"])
        .assert()
        .code(2);
}

#[test]
fn empty_file_is_fine() {
    lockline()
        .args(["check", "tests/fixtures/empty.sql"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("in 1 file (0 statements)"));
}

#[test]
fn color_flag_controls_escape_codes() {
    lockline()
        .args(["check", "tests/fixtures/dangerous", "--color", "always"])
        .assert()
        .stdout(predicate::str::contains("\x1b["));
    lockline()
        .args(["check", "tests/fixtures/dangerous", "--color", "never"])
        .assert()
        .stdout(predicate::str::contains("\x1b[").not());
}

#[test]
fn auto_color_is_off_when_piped() {
    lockline()
        .args(["check", "tests/fixtures/dangerous"])
        .assert()
        .stdout(predicate::str::contains("\x1b[").not());
}

#[test]
fn closed_stdout_does_not_panic() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("lockline"))
        .args(["check", "-", "--color", "never"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let sql = "CREATE INDEX i ON users (email);\n".repeat(2000);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(sql.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_ne!(output.status.code(), Some(101));
}

#[test]
fn empty_directory_warns_but_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    lockline()
        .args(["check", tmp.path().to_str().unwrap()])
        .assert()
        .code(0)
        .stderr(predicate::str::contains("warning: "))
        .stderr(predicate::str::contains("no .sql files found"));
}

#[test]
fn warnings_do_not_fail_by_default() {
    lockline()
        .args(["check", "tests/fixtures/warnings"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 errors, 1 warning in 1 file"));
    lockline()
        .args(["check", "tests/fixtures/warnings", "--fail-on", "warning"])
        .assert()
        .code(1);
}

#[test]
fn rules_lists_every_rule() {
    let output = lockline()
        .arg("rules")
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert_eq!(text.lines().count(), 20);
    assert!(
        text.lines()
            .any(|l| l.starts_with("create-index-not-concurrently") && l.contains(" error "))
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("missing-lock-timeout") && l.contains(" warning "))
    );
}

#[test]
fn explain_prints_docs() {
    lockline()
        .args(["explain", "set-not-null"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("# set-not-null\n"))
        .stdout(predicate::str::contains("## Instead"));
}

#[test]
fn explain_unknown_exits_two() {
    lockline()
        .args(["explain", "no-such-rule"])
        .assert()
        .code(2)
        .stderr("error: unknown rule \"no-such-rule\"; run \"lockline rules\" to list them\n");
}
