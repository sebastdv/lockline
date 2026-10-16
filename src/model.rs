//! Types shared by the catalog, the analyzer and the reports.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::locks::LockMode;

/// A table as PostgreSQL resolves it: the parser already lowercases unquoted names,
/// and an unqualified name is treated as `public.<name>`.
#[derive(Debug, Clone)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
}

impl TableRef {
    pub fn from_range_var(rv: &pg_query::protobuf::RangeVar) -> TableRef {
        TableRef {
            schema: (!rv.schemaname.is_empty()).then(|| rv.schemaname.clone()),
            name: rv.relname.clone(),
        }
    }

    fn key(&self) -> (&str, &str) {
        (self.schema.as_deref().unwrap_or("public"), &self.name)
    }
}

impl PartialEq for TableRef {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for TableRef {}

impl Hash for TableRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

impl fmt::Display for TableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.schema {
            Some(schema) => write!(f, "{schema}.{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// How much work a statement does on a table, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cost {
    Metadata,
    Scan,
    Rewrite,
    Unknown,
}

/// What a statement does to one table.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub table: TableRef,
    pub lock: LockMode,
    pub cost: Cost,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Analysis {
    /// The statement locks at least one existing table.
    Locks(Vec<Effect>),
    /// The statement takes no relevant lock on existing tables (CREATE TABLE, SET, BEGIN…).
    NoTableLock,
    /// The statement is outside the catalog; lockline does not guess.
    NotAnalyzed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
    pub help: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash_of(t: &TableRef) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    fn table(schema: Option<&str>, name: &str) -> TableRef {
        TableRef {
            schema: schema.map(str::to_string),
            name: name.to_string(),
        }
    }

    #[test]
    fn table_ref_public_schema_equals_unqualified() {
        let qualified = table(Some("public"), "users");
        let bare = table(None, "users");
        assert_eq!(qualified, bare);
        assert_eq!(hash_of(&qualified), hash_of(&bare));
        assert_ne!(table(None, "Users"), table(None, "users"));
        assert_ne!(table(Some("app"), "users"), bare);
    }

    #[test]
    fn table_ref_display() {
        assert_eq!(table(Some("app"), "users").to_string(), "app.users");
        assert_eq!(table(None, "users").to_string(), "users");
    }
}
