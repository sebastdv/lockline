//! Reads migration units from files, directories and stdin.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::analyze::{MigrationUnit, Source};

#[derive(Debug, Error)]
pub enum InputError {
    #[error("{path}: no such file or directory")]
    NotFound { path: String },
    #[error("{path}: {source}")]
    Io { path: String, source: io::Error },
    #[error("{path}: {message}")]
    Laravel { path: String, message: String },
}

#[derive(Debug, Default)]
pub struct Collected {
    pub units: Vec<MigrationUnit>,
    /// Non-fatal problems with the input, such as a directory with no SQL files.
    pub warnings: Vec<String>,
}

/// `-` reads stdin. Directories are walked recursively for `*.sql` files (any case), in
/// natural order so `V2__` comes before `V10__`; symlinked directories are not followed.
/// Explicit paths keep the order they were given in.
pub fn collect_units(paths: &[PathBuf], stdin: &mut dyn Read) -> Result<Collected, InputError> {
    let mut units = Vec::new();
    let mut warnings = Vec::new();
    for path in paths {
        if path.as_os_str() == "-" {
            let mut sql = String::new();
            stdin
                .read_to_string(&mut sql)
                .map_err(|source| InputError::Io {
                    path: "<stdin>".into(),
                    source,
                })?;
            units.push(unit("<stdin>".into(), sql));
        } else if path.is_dir() {
            let mut files = Vec::new();
            find_sql_files(path, &mut files)?;
            if files.is_empty() {
                warnings.push(format!("{}: no .sql files found", path.display()));
            }
            files.sort_by(|a, b| natural_cmp(a, b));
            for file in files {
                units.push(read_file(&file)?);
            }
        } else if path.exists() {
            units.push(read_file(path)?);
        } else {
            return Err(InputError::NotFound {
                path: path.display().to_string(),
            });
        }
    }
    Ok(Collected { units, warnings })
}

/// Compares paths component by component; runs of digits compare by numeric value.
pub fn natural_cmp(a: &Path, b: &Path) -> Ordering {
    let mut left = a.components();
    let mut right = b.components();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = natural_cmp_str(
                    &x.as_os_str().to_string_lossy(),
                    &y.as_os_str().to_string_lossy(),
                );
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

fn natural_cmp_str(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a.is_empty(), b.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let (run_a, rest_a) = split_run(a);
        let (run_b, rest_b) = split_run(b);
        let both_digits =
            run_a.as_bytes()[0].is_ascii_digit() && run_b.as_bytes()[0].is_ascii_digit();
        let order = if both_digits {
            let (ta, tb) = (run_a.trim_start_matches('0'), run_b.trim_start_matches('0'));
            ta.len()
                .cmp(&tb.len())
                .then_with(|| ta.cmp(tb))
                .then_with(|| run_a.len().cmp(&run_b.len()))
        } else {
            run_a.cmp(run_b)
        };
        if order != Ordering::Equal {
            return order;
        }
        (a, b) = (rest_a, rest_b);
    }
}

/// Splits off the leading run of digits, or of non-digits.
fn split_run(s: &str) -> (&str, &str) {
    let digits = s.as_bytes()[0].is_ascii_digit();
    let end = s
        .find(|c: char| c.is_ascii_digit() != digits)
        .unwrap_or(s.len());
    s.split_at(end)
}

fn find_sql_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), InputError> {
    let entries = fs::read_dir(dir).map_err(|e| io_error(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_error(dir, e))?;
        let path = entry.path();
        let is_symlink = entry.file_type().is_ok_and(|t| t.is_symlink());
        if path.is_dir() {
            if !is_symlink {
                find_sql_files(&path, out)?;
            }
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<MigrationUnit, InputError> {
    let sql = fs::read_to_string(path).map_err(|e| io_error(path, e))?;
    Ok(unit(path.display().to_string(), sql))
}

fn unit(name: String, sql: String) -> MigrationUnit {
    let sql = match sql.strip_prefix('\u{FEFF}') {
        Some(rest) => rest.to_string(),
        None => sql,
    };
    MigrationUnit::new(name, sql)
}

fn io_error(path: &Path, source: io::Error) -> InputError {
    InputError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Migrations that already ran, read only to learn the schema (`--history`).
///
/// - A directory: its `.sql` files that sort before the first analyzed file (natural order by
///   file name), skipping the analyzed files themselves. In CI this lets you check only the
///   files a pull request adds, with the context of the ones before it.
/// - A file: all of it, for example a `pg_dump --schema-only` or Laravel's
///   `php artisan schema:dump` (`database/schema/pgsql-schema.sql`).
///
/// `COPY ... FROM stdin` data blocks and psql meta-commands (`\restrict`, `\connect`) that
/// dumps contain are removed: they are not SQL the parser accepts.
pub fn collect_history(path: &Path, analyzed: &[MigrationUnit]) -> Result<Collected, InputError> {
    if !path.exists() {
        return Err(InputError::NotFound {
            path: path.display().to_string(),
        });
    }
    let mut collected = collect_units(&[path.to_path_buf()], &mut io::empty())?;
    if path.is_dir() {
        let analyzed_paths: HashSet<PathBuf> = analyzed
            .iter()
            .filter(|u| u.source == Source::Sql)
            .filter_map(|u| fs::canonicalize(&u.name).ok())
            .collect();
        let first = analyzed
            .iter()
            .filter(|u| u.name != "<stdin>")
            .filter_map(|u| Path::new(&u.name).file_name().map(PathBuf::from))
            .min_by(|a, b| natural_cmp(a, b));
        collected.units.retain(|unit| {
            let resolved = fs::canonicalize(&unit.name).ok();
            let analyzed_too = resolved.is_some_and(|p| analyzed_paths.contains(&p));
            let earlier = match (&first, Path::new(&unit.name).file_name()) {
                (Some(first), Some(name)) => natural_cmp(Path::new(name), first) == Ordering::Less,
                _ => true,
            };
            !analyzed_too && earlier
        });
    }
    for unit in &mut collected.units {
        unit.sql = strip_dump_noise(&unit.sql);
    }
    Ok(collected)
}

/// Removes `COPY ... FROM stdin;` data (up to the `\.` line) and psql meta-commands.
fn strip_dump_noise(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut in_copy = false;
    for line in sql.lines() {
        if in_copy {
            if line == "\\." {
                in_copy = false;
            }
            out.push('\n');
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('\\') {
            out.push('\n');
            continue;
        }
        if trimmed.len() >= 5
            && trimmed[..5].eq_ignore_ascii_case("copy ")
            && trimmed
                .trim_end()
                .to_ascii_lowercase()
                .ends_with("from stdin;")
        {
            in_copy = true;
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
