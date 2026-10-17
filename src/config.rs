//! `lockline.toml`: project-wide settings, overridden by command-line flags.
//!
//! ```toml
//! transaction = "per-unit"     # or "none"
//! fail_on = "error"            # or "warning"
//! exclude = ["migrations/legacy/**"]
//! [rules]
//! drop-object = "off"          # off | warning | error
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use thiserror::Error;

use crate::analyze::TransactionMode;
use crate::model::Severity;
use crate::rules::infos;

pub const FILE_NAME: &str = "lockline.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleLevel {
    Off,
    Warning,
    Error,
}

#[derive(Debug, Default)]
pub struct FileConfig {
    pub transaction: Option<TransactionMode>,
    pub fail_on: Option<Severity>,
    pub exclude: Vec<String>,
    pub rules: BTreeMap<String, RuleLevel>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
    #[error("{path}: unknown rule \"{rule}\" in [rules]; run \"lockline rules\" to list them")]
    UnknownRule { path: String, rule: String },
    #[error("invalid exclude pattern \"{pattern}\": {message}")]
    BadGlob { pattern: String, message: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    transaction: Option<TransactionSetting>,
    fail_on: Option<FailOnSetting>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    rules: BTreeMap<String, RuleLevel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TransactionSetting {
    PerUnit,
    None,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FailOnSetting {
    Error,
    Warning,
}

/// `lockline.toml` in `start` or the closest parent directory.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(FILE_NAME))
        .find(|path| path.is_file())
}

pub fn load_config(path: &Path) -> Result<FileConfig, ConfigError> {
    let display = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: display.clone(),
        source,
    })?;
    let raw: RawConfig = toml::from_str(&text).map_err(|e| ConfigError::Invalid {
        path: display.clone(),
        // Display includes the line, the column and the offending key.
        message: e.to_string().trim_end().to_string(),
    })?;
    let known = infos();
    if let Some(rule) = raw
        .rules
        .keys()
        .find(|id| !known.iter().any(|i| i.id == *id))
    {
        return Err(ConfigError::UnknownRule {
            path: display,
            rule: rule.clone(),
        });
    }
    Ok(FileConfig {
        transaction: raw.transaction.map(|t| match t {
            TransactionSetting::PerUnit => TransactionMode::PerUnit,
            TransactionSetting::None => TransactionMode::None,
        }),
        fail_on: raw.fail_on.map(|f| match f {
            FailOnSetting::Error => Severity::Error,
            FailOnSetting::Warning => Severity::Warning,
        }),
        exclude: raw.exclude,
        rules: raw.rules,
    })
}

pub fn exclude_set(patterns: &[String]) -> Result<GlobSet, ConfigError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|e| ConfigError::BadGlob {
            pattern: pattern.clone(),
            message: e.kind().to_string(),
        })?;
        builder.add(glob);
    }
    builder.build().map_err(|e| ConfigError::BadGlob {
        pattern: patterns.join(", "),
        message: e.to_string(),
    })
}
