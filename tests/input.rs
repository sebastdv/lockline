use std::fs;
use std::path::{Path, PathBuf};

use lockline::input::{InputError, collect_units};

fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn names_relative_to(dir: &Path, paths: &[PathBuf]) -> Vec<String> {
    let units = collect_units(paths, &mut std::io::empty()).unwrap();
    let prefix = format!("{}/", dir.display());
    units
        .into_iter()
        .map(|u| u.name.strip_prefix(&prefix).unwrap_or(&u.name).to_string())
        .collect()
}

#[test]
fn directory_is_recursive_sorted_and_sql_only() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path().join("m");
    write(&m, "0002_b.sql", b"SELECT 2;");
    write(&m, "0001_a.sql", b"SELECT 1;");
    write(&m, "README.md", b"# not sql");
    write(&m, "2026_x/migration.sql", b"SELECT 3;");
    write(&m, "2026_x/notes.txt", b"nope");
    assert_eq!(
        names_relative_to(tmp.path(), &[m]),
        ["m/0001_a.sql", "m/0002_b.sql", "m/2026_x/migration.sql"]
    );
}

#[test]
fn explicit_paths_keep_given_order() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "b.sql", b"SELECT 2;");
    write(tmp.path(), "a.sql", b"SELECT 1;");
    write(tmp.path(), "c.txt", b"SELECT 3;");
    let paths = ["b.sql", "a.sql", "c.txt"].map(|p| tmp.path().join(p));
    assert_eq!(
        names_relative_to(tmp.path(), &paths),
        ["b.sql", "a.sql", "c.txt"]
    );
}

#[test]
fn dash_reads_stdin() {
    let units = collect_units(&["-".into()], &mut "SELECT 1;".as_bytes()).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].name, "<stdin>");
    assert_eq!(units[0].sql, "SELECT 1;");
}

#[test]
fn bom_is_stripped() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "bom.sql", b"\xEF\xBB\xBFSELECT 1;");
    let units = collect_units(&[tmp.path().join("bom.sql")], &mut std::io::empty()).unwrap();
    assert_eq!(units[0].sql, "SELECT 1;");
}

#[test]
fn missing_path_is_not_found() {
    let err = collect_units(&["does/not/exist.sql".into()], &mut std::io::empty()).unwrap_err();
    assert!(matches!(err, InputError::NotFound { .. }));
    assert_eq!(
        err.to_string(),
        "does/not/exist.sql: no such file or directory"
    );
}

#[test]
fn invalid_utf8_is_an_io_error() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "latin1.sql", b"SELECT '\xF1';");
    let err = collect_units(&[tmp.path().join("latin1.sql")], &mut std::io::empty()).unwrap_err();
    assert!(matches!(err, InputError::Io { .. }));
}
