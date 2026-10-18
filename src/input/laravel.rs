//! Reads the output of `php artisan migrate --pretend`: one migration unit per Laravel
//! migration, with the SQL Laravel would run.
//!
//! Two formats are understood:
//!
//! - Laravel 9 and later: a line with the migration name followed by dots, then one line per
//!   statement starting with `⇂`. A statement with line breaks continues on the following
//!   lines until the next `⇂` or migration.
//! - Laravel 8 and earlier: `ClassName: statement` on every line.
//!
//! ANSI color codes (`--ansi`) and the `INFO` lines are ignored.

use std::io::Read;
use std::path::PathBuf;

use crate::analyze::{MigrationUnit, Source};

use super::files::{Collected, InputError};

/// Default folder of Laravel migrations, used to name units after their PHP file so CI
/// annotations land on it.
pub const DEFAULT_MIGRATIONS_DIR: &str = "database/migrations";

struct Pending {
    name: String,
    statements: Vec<String>,
}

/// Parses `migrate --pretend` output. `migrations_dir` names each unit
/// `<migrations_dir>/<migration>.php` (Laravel 9+); Laravel 8 output only has class names, so
/// those units keep the class name.
pub fn parse_pretend(output: &str, migrations_dir: &str) -> Vec<MigrationUnit> {
    let mut units: Vec<Pending> = Vec::new();
    let mut in_statement = false;
    for raw in strip_ansi(output).lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            in_statement = false;
            continue;
        }
        if let Some(name) = migration_header(trimmed) {
            units.push(Pending {
                name: format!("{}/{name}.php", migrations_dir.trim_end_matches('/')),
                statements: Vec::new(),
            });
            in_statement = false;
        } else if let Some(sql) = trimmed.strip_prefix('⇂') {
            if let Some(unit) = units.last_mut() {
                unit.statements.push(sql.trim().to_string());
                in_statement = true;
            }
        } else if let Some((class, sql)) = legacy_line(trimmed) {
            match units.last_mut() {
                Some(unit) if unit.name == class => unit.statements.push(sql.to_string()),
                _ => units.push(Pending {
                    name: class.to_string(),
                    statements: vec![sql.to_string()],
                }),
            }
            in_statement = true;
        } else if in_statement {
            // A statement with line breaks: Laravel prints the rest as is.
            if let Some(statement) = units.last_mut().and_then(|u| u.statements.last_mut()) {
                statement.push('\n');
                statement.push_str(trimmed);
            }
        }
    }
    units
        .into_iter()
        .map(|unit| MigrationUnit {
            name: unit.name,
            sql: unit
                .statements
                .iter()
                .map(|s| {
                    let s = s.trim_end();
                    if s.ends_with(';') {
                        s.to_string()
                    } else {
                        format!("{s};")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            source: Source::Laravel,
        })
        .collect()
}

/// `2026_10_17_000000_add_nickname_to_users ......` → the migration name.
fn migration_header(line: &str) -> Option<&str> {
    let (name, rest) = line.split_once(' ')?;
    let dotted = rest.trim_start().starts_with("...");
    let looks_like_migration = name.len() > 18
        && name.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (dotted && looks_like_migration).then_some(name)
}

/// `CreateUsersTable: create table ...` (Laravel 8 and earlier).
fn legacy_line(line: &str) -> Option<(&str, &str)> {
    let (class, sql) = line.split_once(": ")?;
    let is_class = class.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && class.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (is_class && !sql.trim().is_empty()).then(|| (class, sql.trim()))
}

/// Removes ANSI escape sequences (`ESC [ ... letter`).
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Reads `migrate --pretend` output from files or stdin (`-`).
pub fn collect_laravel(
    paths: &[PathBuf],
    stdin: &mut dyn Read,
    migrations_dir: &str,
) -> Result<Collected, InputError> {
    let mut collected = Collected::default();
    for path in paths {
        let (label, output) = if path.as_os_str() == "-" {
            let mut text = String::new();
            stdin
                .read_to_string(&mut text)
                .map_err(|source| InputError::Io {
                    path: "<stdin>".into(),
                    source,
                })?;
            ("<stdin>".to_string(), text)
        } else if path.is_dir() {
            return Err(InputError::Laravel {
                path: path.display().to_string(),
                message: "is a directory; pass the saved output of `php artisan migrate --pretend`, or `-` to read it from a pipe".into(),
            });
        } else if path.exists() {
            let text = std::fs::read_to_string(path).map_err(|source| InputError::Io {
                path: path.display().to_string(),
                source,
            })?;
            (path.display().to_string(), text)
        } else {
            return Err(InputError::NotFound {
                path: path.display().to_string(),
            });
        };
        let units = parse_pretend(&output, migrations_dir);
        if units.is_empty() {
            if output.contains("Nothing to migrate") {
                collected
                    .warnings
                    .push(format!("{label}: Laravel had nothing to migrate"));
            } else {
                return Err(InputError::Laravel {
                    path: label,
                    message: "no migrations found; expected the output of `php artisan migrate --pretend`".into(),
                });
            }
        }
        collected.units.extend(units);
    }
    Ok(collected)
}
