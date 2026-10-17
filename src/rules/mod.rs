//! Lint rules. Each rule looks at one statement in context and may report one finding.

mod add_check_without_not_valid;
mod add_column_not_null_without_default;
mod add_column_rewrite;
mod add_column_unknown_default;
mod add_foreign_key_without_not_valid;
mod add_unique_without_index;
mod alter_column_type;
mod backfill_in_migration;
mod cannot_run_in_transaction;
mod constraints;
mod create_index_not_concurrently;
mod drop_index_not_concurrently;
mod drop_object;
mod missing_lock_timeout;
mod refresh_matview_not_concurrently;
mod reindex_not_concurrently;
mod rename_object;
mod rewrite_table;
mod set_not_null;

use pg_query::NodeEnum;
use pg_query::protobuf::{AlterTableCmd, AlterTableType, ColumnDef, Constraint, ObjectType};

use crate::model::{Analysis, Effect, Finding, Severity};
use crate::parse::Statement;
use crate::schema::Schema;

pub struct RuleCtx<'a> {
    pub stmt: &'a Statement,
    pub analysis: &'a Analysis,
    /// Every table the statement locks was created earlier in the same unit.
    pub new_table: bool,
    pub in_transaction: bool,
    pub schema: &'a Schema,
    /// A non-zero lock_timeout is in effect.
    pub lock_timeout_set: bool,
}

pub trait Rule {
    fn id(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn check(&self, ctx: &RuleCtx) -> Option<Finding>;

    /// Report only the first finding of this rule in each unit.
    fn once_per_unit(&self) -> bool {
        false
    }

    /// One-line description (from the rule's documentation).
    fn summary(&self) -> &'static str {
        summary_of(docs_for(self.id()))
    }

    /// The rule's documentation page, `docs/rules/<id>.md`.
    fn docs(&self) -> &'static str {
        docs_for(self.id())
    }

    fn finding(&self, message: String, help: Option<String>) -> Finding {
        Finding {
            rule: self.id(),
            severity: self.severity(),
            message,
            help,
            line: None,
        }
    }
}

pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(create_index_not_concurrently::CreateIndexNotConcurrently),
        Box::new(cannot_run_in_transaction::CannotRunInTransaction),
        Box::new(add_column_rewrite::AddColumnRewrite),
        Box::new(drop_index_not_concurrently::DropIndexNotConcurrently),
        Box::new(reindex_not_concurrently::ReindexNotConcurrently),
        Box::new(rewrite_table::RewriteTable),
        Box::new(refresh_matview_not_concurrently::RefreshMatviewNotConcurrently),
        Box::new(backfill_in_migration::BackfillInMigration),
        Box::new(add_column_not_null_without_default::AddColumnNotNullWithoutDefault),
        Box::new(add_column_unknown_default::AddColumnUnknownDefault),
        Box::new(alter_column_type::AlterColumnType),
        Box::new(set_not_null::SetNotNull),
        Box::new(add_check_without_not_valid::AddCheckWithoutNotValid),
        Box::new(add_foreign_key_without_not_valid::AddForeignKeyWithoutNotValid),
        Box::new(add_unique_without_index::AddUniqueWithoutIndex),
        Box::new(missing_lock_timeout::MissingLockTimeout),
        Box::new(rename_object::RenameObject),
        Box::new(drop_object::DropObject),
    ]
}

impl RuleCtx<'_> {
    /// The relation the statement changes (`effects[0]`), unless it was created in this unit.
    pub fn existing_target(&self) -> Option<&Effect> {
        match self.analysis {
            Analysis::Locks(effects) => effects.first().filter(|e| !e.new_table),
            _ => None,
        }
    }

    /// Every table the statement locks that was not created in this unit, in order.
    pub fn existing_targets(&self) -> Vec<&Effect> {
        match self.analysis {
            Analysis::Locks(effects) => effects.iter().filter(|e| !e.new_table).collect(),
            _ => Vec::new(),
        }
    }
}

/// `"a", "b"`: table names for a message.
pub(crate) fn quoted_names(effects: &[&Effect]) -> String {
    effects
        .iter()
        .map(|e| format!("\"{}\"", e.table.plain()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Subcommands of an `ALTER TABLE` on a table (not a view, index, type…).
pub(crate) fn table_alter_cmds(node: &NodeEnum) -> Vec<&AlterTableCmd> {
    let NodeEnum::AlterTableStmt(stmt) = node else {
        return Vec::new();
    };
    if stmt.objtype != ObjectType::ObjectTable as i32 {
        return Vec::new();
    }
    stmt.cmds
        .iter()
        .filter_map(|c| match c.node.as_ref() {
            Some(NodeEnum::AlterTableCmd(cmd)) => Some(&**cmd),
            _ => None,
        })
        .collect()
}

/// The column added by an `ADD COLUMN` subcommand.
pub(crate) fn added_column(cmd: &AlterTableCmd) -> Option<&ColumnDef> {
    if cmd.subtype != AlterTableType::AtAddColumn as i32 {
        return None;
    }
    match cmd.def.as_ref()?.node.as_ref()? {
        NodeEnum::ColumnDef(column) => Some(column),
        _ => None,
    }
}

pub(crate) fn column_constraints(column: &ColumnDef) -> impl Iterator<Item = &Constraint> {
    column
        .constraints
        .iter()
        .filter_map(|c| match c.node.as_ref() {
            Some(NodeEnum::Constraint(c)) => Some(&**c),
            _ => None,
        })
}

/// A rule as the `rules` and `explain` commands show it.
#[derive(Debug, Clone, Copy)]
pub struct RuleInfo {
    pub id: &'static str,
    pub severity: Severity,
    pub summary: &'static str,
    pub docs: &'static str,
}

/// Every rule, in registration order, followed by the findings the analyzer itself raises.
pub fn infos() -> Vec<RuleInfo> {
    let mut infos: Vec<RuleInfo> = all()
        .iter()
        .map(|rule| RuleInfo {
            id: rule.id(),
            severity: rule.severity(),
            summary: rule.summary(),
            docs: rule.docs(),
        })
        .collect();
    for (id, severity) in [
        ("parse-error", Severity::Error),
        ("ignore-without-reason", Severity::Warning),
        ("ignore-unknown-rule", Severity::Warning),
    ] {
        let docs = docs_for(id);
        infos.push(RuleInfo {
            id,
            severity,
            summary: summary_of(docs),
            docs,
        });
    }
    infos
}

/// The documentation pages are the single source for summaries and `lockline explain`.
fn docs_for(id: &str) -> &'static str {
    match id {
        "create-index-not-concurrently" => {
            include_str!("../../docs/rules/create-index-not-concurrently.md")
        }
        "drop-index-not-concurrently" => {
            include_str!("../../docs/rules/drop-index-not-concurrently.md")
        }
        "reindex-not-concurrently" => include_str!("../../docs/rules/reindex-not-concurrently.md"),
        "cannot-run-in-transaction" => {
            include_str!("../../docs/rules/cannot-run-in-transaction.md")
        }
        "add-column-rewrite" => include_str!("../../docs/rules/add-column-rewrite.md"),
        "add-column-not-null-without-default" => {
            include_str!("../../docs/rules/add-column-not-null-without-default.md")
        }
        "add-column-unknown-default" => {
            include_str!("../../docs/rules/add-column-unknown-default.md")
        }
        "alter-column-type" => include_str!("../../docs/rules/alter-column-type.md"),
        "set-not-null" => include_str!("../../docs/rules/set-not-null.md"),
        "add-check-without-not-valid" => {
            include_str!("../../docs/rules/add-check-without-not-valid.md")
        }
        "add-foreign-key-without-not-valid" => {
            include_str!("../../docs/rules/add-foreign-key-without-not-valid.md")
        }
        "add-unique-without-index" => include_str!("../../docs/rules/add-unique-without-index.md"),
        "rewrite-table" => include_str!("../../docs/rules/rewrite-table.md"),
        "refresh-matview-not-concurrently" => {
            include_str!("../../docs/rules/refresh-matview-not-concurrently.md")
        }
        "missing-lock-timeout" => include_str!("../../docs/rules/missing-lock-timeout.md"),
        "rename-object" => include_str!("../../docs/rules/rename-object.md"),
        "drop-object" => include_str!("../../docs/rules/drop-object.md"),
        "backfill-in-migration" => include_str!("../../docs/rules/backfill-in-migration.md"),
        "parse-error" => include_str!("../../docs/rules/parse-error.md"),
        "ignore-without-reason" => include_str!("../../docs/rules/ignore-without-reason.md"),
        "ignore-unknown-rule" => include_str!("../../docs/rules/ignore-unknown-rule.md"),
        _ => "",
    }
}

/// The first paragraph after the `**Severity:**` line.
fn summary_of(docs: &'static str) -> &'static str {
    docs.lines()
        .skip_while(|line| !line.starts_with("**Severity:**"))
        .skip(1)
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
}
