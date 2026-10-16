//! Reads migration units from files, directories and stdin.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::analyze::MigrationUnit;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("{path}: no such file or directory")]
    NotFound { path: String },
    #[error("{path}: {source}")]
    Io { path: String, source: io::Error },
}

/// `-` reads stdin. Directories are walked recursively for `*.sql` files, sorted by path
/// (so numbered or timestamped migrations come out in order). Explicit paths keep the
/// order they were given in.
pub fn collect_units(
    paths: &[PathBuf],
    stdin: &mut dyn Read,
) -> Result<Vec<MigrationUnit>, InputError> {
    let mut units = Vec::new();
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
            files.sort();
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
    Ok(units)
}

fn find_sql_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), InputError> {
    let entries = fs::read_dir(dir).map_err(|e| io_error(dir, e))?;
    for entry in entries {
        let path = entry.map_err(|e| io_error(dir, e))?.path();
        if path.is_dir() {
            find_sql_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "sql") {
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
    MigrationUnit { name, sql }
}

fn io_error(path: &Path, source: io::Error) -> InputError {
    InputError::Io {
        path: path.display().to_string(),
        source,
    }
}
