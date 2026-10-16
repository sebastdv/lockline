use std::fs;
use std::path::{Path, PathBuf};

use lockline::input::{InputError, collect_units, natural_cmp};

fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn names_relative_to(dir: &Path, paths: &[PathBuf]) -> Vec<String> {
    let units = collect_units(paths, &mut std::io::empty()).unwrap().units;
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
    let units = collect_units(&["-".into()], &mut "SELECT 1;".as_bytes())
        .unwrap()
        .units;
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].name, "<stdin>");
    assert_eq!(units[0].sql, "SELECT 1;");
}

#[test]
fn bom_is_stripped() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "bom.sql", b"\xEF\xBB\xBFSELECT 1;");
    let units = collect_units(&[tmp.path().join("bom.sql")], &mut std::io::empty())
        .unwrap()
        .units;
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

#[test]
fn flyway_versions_sort_naturally() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path().join("m");
    for name in ["V10__c.sql", "V2__b.sql", "V1__a.sql"] {
        write(&m, name, b"SELECT 1;");
    }
    assert_eq!(
        names_relative_to(tmp.path(), &[m]),
        ["m/V1__a.sql", "m/V2__b.sql", "m/V10__c.sql"]
    );
}

#[test]
fn natural_cmp_cases() {
    use std::cmp::Ordering::*;
    let cmp = |a: &str, b: &str| natural_cmp(Path::new(a), Path::new(b));
    assert_eq!(cmp("0002", "0010"), Less);
    assert_eq!(cmp("a2", "a10"), Less);
    assert_eq!(cmp("abc", "abc"), Equal);
    assert_eq!(cmp("m/2026_x/migration.sql", "m/0002_b.sql"), Greater);
    assert_eq!(cmp("m/0001_a.sql", "m/0001_b.sql"), Less);
}

#[test]
fn uppercase_extension_is_included() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path().join("m");
    write(&m, "0001.SQL", b"SELECT 1;");
    assert_eq!(names_relative_to(tmp.path(), &[m]), ["m/0001.SQL"]);
}

#[cfg(unix)]
#[test]
fn symlinked_directory_loop_is_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path().join("m");
    write(&m, "a.sql", b"SELECT 1;");
    fs::create_dir_all(m.join("sub")).unwrap();
    std::os::unix::fs::symlink("..", m.join("sub/back")).unwrap();
    assert_eq!(names_relative_to(tmp.path(), &[m]), ["m/a.sql"]);
}

#[test]
fn empty_directory_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path().join("m");
    write(&m, "README.md", b"nothing");
    let collected = collect_units(std::slice::from_ref(&m), &mut std::io::empty()).unwrap();
    assert!(collected.units.is_empty());
    assert_eq!(
        collected.warnings,
        [format!("{}: no .sql files found", m.display())]
    );
}
