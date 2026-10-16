//! Maps a parsed statement to the locks it takes and the work it does on each table.
//!
//! Milestone 1 covers indexes and `ALTER TABLE ... ADD COLUMN`; everything else is reported
//! as `NotAnalyzed` rather than guessed.

use pg_query::NodeEnum;
use pg_query::protobuf::{
    AlterTableCmd, AlterTableStmt, AlterTableType, ColumnDef, ConstrType, CreateStmt,
};

use crate::locks::LockMode;
use crate::model::{Analysis, Cost, Effect, TableRef};
use crate::volatility::{Volatility, classify_default, last_name};

const SERIAL_TYPES: &[&str] = &[
    "serial",
    "bigserial",
    "smallserial",
    "serial2",
    "serial4",
    "serial8",
];

pub fn analyze_statement(node: &NodeEnum) -> Analysis {
    match node {
        NodeEnum::IndexStmt(stmt) => {
            let Some(relation) = &stmt.relation else {
                return Analysis::NotAnalyzed;
            };
            let (lock, note) = if stmt.concurrent {
                (
                    LockMode::ShareUpdateExclusive,
                    "index build, does not block writes",
                )
            } else {
                (LockMode::Share, "index build")
            };
            Analysis::Locks(vec![Effect {
                table: TableRef::from_range_var(relation),
                lock,
                cost: Cost::Scan,
                note: Some(note.to_string()),
            }])
        }
        NodeEnum::AlterTableStmt(stmt) => analyze_alter_table(stmt),
        // These lock existing relations (the parent, the referenced table, the old view);
        // milestone 2 models them. Until then lockline does not claim they are lock-free.
        NodeEnum::CreateStmt(stmt) if locks_other_tables(stmt) => Analysis::NotAnalyzed,
        NodeEnum::ViewStmt(stmt) if stmt.replace => Analysis::NotAnalyzed,
        NodeEnum::CreateStmt(_)
        | NodeEnum::TransactionStmt(_)
        | NodeEnum::VariableSetStmt(_)
        | NodeEnum::CreateFunctionStmt(_)
        | NodeEnum::ViewStmt(_)
        | NodeEnum::CommentStmt(_)
        | NodeEnum::CreateEnumStmt(_) => Analysis::NoTableLock,
        _ => Analysis::NotAnalyzed,
    }
}

/// Statements PostgreSQL refuses to run inside a transaction block, with the label to show.
pub fn forbidden_in_transaction(node: &NodeEnum) -> Option<&'static str> {
    match node {
        NodeEnum::IndexStmt(stmt) if stmt.concurrent => Some("CREATE INDEX CONCURRENTLY"),
        NodeEnum::DropStmt(stmt) if stmt.concurrent => Some("DROP INDEX CONCURRENTLY"),
        NodeEnum::ReindexStmt(stmt)
            if stmt.params.iter().any(|p| {
                matches!(p.node.as_ref(), Some(NodeEnum::DefElem(d)) if d.defname == "concurrently")
            }) =>
        {
            Some("REINDEX CONCURRENTLY")
        }
        NodeEnum::AlterTableStmt(stmt) if alter_cmds(stmt).any(is_detach_concurrently) => {
            Some("DETACH PARTITION CONCURRENTLY")
        }
        NodeEnum::VacuumStmt(stmt) if stmt.is_vacuumcmd => Some("VACUUM"),
        _ => None,
    }
}

/// `PARTITION OF`, `INHERITS` and foreign keys lock the parent or referenced table.
fn locks_other_tables(stmt: &CreateStmt) -> bool {
    stmt.partbound.is_some()
        || !stmt.inh_relations.is_empty()
        || stmt
            .table_elts
            .iter()
            .chain(&stmt.constraints)
            .any(|elt| match elt.node.as_ref() {
                Some(NodeEnum::Constraint(c)) => c.contype == ConstrType::ConstrForeign as i32,
                Some(NodeEnum::ColumnDef(column)) => {
                    constraint_types(column).any(|t| t == ConstrType::ConstrForeign as i32)
                }
                _ => false,
            })
}

fn analyze_alter_table(stmt: &AlterTableStmt) -> Analysis {
    let Some(relation) = &stmt.relation else {
        return Analysis::NotAnalyzed;
    };
    let mut cost = Cost::Metadata;
    let mut commands = 0;
    for cmd in alter_cmds(stmt) {
        commands += 1;
        if cmd.subtype != AlterTableType::AtAddColumn as i32 {
            return Analysis::NotAnalyzed;
        }
        let Some(NodeEnum::ColumnDef(column)) = cmd.def.as_ref().and_then(|d| d.node.as_ref())
        else {
            return Analysis::NotAnalyzed;
        };
        let Some(column_cost) = add_column_cost(column) else {
            return Analysis::NotAnalyzed;
        };
        cost = cost.max(column_cost);
    }
    if commands == 0 {
        return Analysis::NotAnalyzed;
    }
    Analysis::Locks(vec![Effect {
        table: TableRef::from_range_var(relation),
        lock: LockMode::AccessExclusive,
        cost,
        note: None,
    }])
}

/// Inline constraints that validate rows, build an index or lock another table.
/// Milestone 2 models them; until then the whole ALTER TABLE is `NotAnalyzed`.
const UNMODELED_COLUMN_CONSTRAINTS: &[ConstrType] = &[
    ConstrType::ConstrCheck,
    ConstrType::ConstrPrimary,
    ConstrType::ConstrUnique,
    ConstrType::ConstrExclusion,
    ConstrType::ConstrForeign,
];

/// `None` when the column carries a constraint lockline does not model yet.
fn add_column_cost(column: &ColumnDef) -> Option<Cost> {
    if constraint_types(column).any(|t| UNMODELED_COLUMN_CONSTRAINTS.iter().any(|u| *u as i32 == t))
    {
        return None;
    }
    let type_name = column
        .type_name
        .as_ref()
        .map_or("", |t| last_name(&t.names));
    if SERIAL_TYPES.contains(&type_name) {
        return Some(Cost::Rewrite);
    }
    let mut defaults = column.raw_default.iter().map(|b| &**b).collect::<Vec<_>>();
    for constraint in &column.constraints {
        let Some(NodeEnum::Constraint(c)) = constraint.node.as_ref() else {
            continue;
        };
        if c.contype == ConstrType::ConstrIdentity as i32
            || c.contype == ConstrType::ConstrGenerated as i32
        {
            return Some(Cost::Rewrite);
        }
        if c.contype == ConstrType::ConstrDefault as i32 {
            defaults.extend(c.raw_expr.as_deref());
        }
    }
    if defaults
        .into_iter()
        .any(|d| classify_default(d) == Volatility::Volatile)
    {
        Some(Cost::Rewrite)
    } else {
        Some(Cost::Metadata)
    }
}

fn constraint_types(column: &ColumnDef) -> impl Iterator<Item = i32> + '_ {
    column
        .constraints
        .iter()
        .filter_map(|c| match c.node.as_ref() {
            Some(NodeEnum::Constraint(c)) => Some(c.contype),
            _ => None,
        })
}

fn alter_cmds(stmt: &AlterTableStmt) -> impl Iterator<Item = &AlterTableCmd> {
    stmt.cmds.iter().filter_map(|c| match c.node.as_ref() {
        Some(NodeEnum::AlterTableCmd(cmd)) => Some(&**cmd),
        _ => None,
    })
}

fn is_detach_concurrently(cmd: &AlterTableCmd) -> bool {
    cmd.subtype == AlterTableType::AtDetachPartition as i32
        && matches!(
            cmd.def.as_ref().and_then(|d| d.node.as_ref()),
            Some(NodeEnum::PartitionCmd(p)) if p.concurrent
        )
}
