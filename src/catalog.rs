//! Maps a parsed statement to the locks it takes and the work it does on each table.
//!
//! Anything outside the catalog is reported as `NotAnalyzed` rather than guessed.
//! `effects[0]` is always the relation the statement changes; other tables follow.

use pg_query::NodeEnum;
use pg_query::protobuf::{
    AlterTableCmd, AlterTableStmt, AlterTableType, ColumnDef, ConstrType, Constraint, CreateStmt,
    ObjectType,
};

use crate::locks::LockMode;
use crate::model::{Analysis, Cost, Effect, TableRef};
use crate::schema::{Schema, TypeSpec, is_safe_widening};
use crate::volatility::{Volatility, classify_default, last_name};

const SERIAL_TYPES: &[&str] = &[
    "serial",
    "bigserial",
    "smallserial",
    "serial2",
    "serial4",
    "serial8",
];

pub fn analyze_statement(node: &NodeEnum, schema: &Schema) -> Analysis {
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
            Analysis::Locks(vec![
                Effect::new(TableRef::from_range_var(relation), lock, Cost::Scan).with_note(note),
            ])
        }
        NodeEnum::AlterTableStmt(stmt) => analyze_alter_table(stmt, schema),
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

fn analyze_alter_table(stmt: &AlterTableStmt, schema: &Schema) -> Analysis {
    if stmt.objtype != ObjectType::ObjectTable as i32 {
        return Analysis::NotAnalyzed;
    }
    let Some(relation) = &stmt.relation else {
        return Analysis::NotAnalyzed;
    };
    let table = TableRef::from_range_var(relation);
    let mut main = Effect::new(table.clone(), LockMode::AccessShare, Cost::Metadata);
    let mut extra: Vec<Effect> = Vec::new();
    let mut commands = 0;
    for cmd in alter_cmds(stmt) {
        commands += 1;
        let Some(step) = alter_cmd_effect(cmd, &table, schema) else {
            return Analysis::NotAnalyzed;
        };
        main.lock = main.lock.max(step.lock);
        if step.cost > main.cost || (step.cost == main.cost && main.note.is_none()) {
            main.cost = step.cost;
            main.note = step.note.map(str::to_string).or(main.note);
        }
        for effect in step.extra {
            match extra.iter_mut().find(|e| e.table == effect.table) {
                Some(existing) => existing.lock = existing.lock.max(effect.lock),
                None => extra.push(effect),
            }
        }
    }
    if commands == 0 {
        return Analysis::NotAnalyzed;
    }
    let mut effects = vec![main];
    effects.extend(extra);
    Analysis::Locks(effects)
}

/// What one ALTER TABLE subcommand does to the altered table, plus locks on other tables.
struct CmdEffect {
    lock: LockMode,
    cost: Cost,
    note: Option<&'static str>,
    extra: Vec<Effect>,
}

impl CmdEffect {
    fn new(lock: LockMode, cost: Cost) -> Self {
        CmdEffect {
            lock,
            cost,
            note: None,
            extra: Vec::new(),
        }
    }

    fn note(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }
}

/// `None` for subcommands outside the catalog: the whole statement is then not analyzed.
fn alter_cmd_effect(cmd: &AlterTableCmd, table: &TableRef, schema: &Schema) -> Option<CmdEffect> {
    use AlterTableType::*;
    use LockMode::*;
    let def = cmd.def.as_ref().and_then(|d| d.node.as_ref());
    let effect = match AlterTableType::try_from(cmd.subtype).ok()? {
        AtAddColumn => {
            let Some(NodeEnum::ColumnDef(column)) = def else {
                return None;
            };
            add_column_effect(column)
        }
        AtDropColumn | AtDropNotNull | AtColumnDefault | AtDropConstraint => {
            CmdEffect::new(AccessExclusive, Cost::Metadata)
        }
        AtAlterColumnType => {
            let Some(NodeEnum::ColumnDef(column)) = def else {
                return None;
            };
            let new_type = column.type_name.as_ref().map(TypeSpec::from_type_name);
            let widening = column.raw_default.is_none()
                && match (schema.column_type(table, &cmd.name), &new_type) {
                    (Some(old), Some(new)) => is_safe_widening(old, new),
                    _ => false,
                };
            let cost = if widening {
                Cost::Metadata
            } else {
                Cost::Rewrite
            };
            CmdEffect::new(AccessExclusive, cost)
        }
        AtSetNotNull => {
            let cost = if schema.has_validated_not_null_check(table, &cmd.name) {
                Cost::Metadata
            } else {
                Cost::Scan
            };
            CmdEffect::new(AccessExclusive, cost)
        }
        AtAddConstraint => {
            let Some(NodeEnum::Constraint(c)) = def else {
                return None;
            };
            add_constraint_effect(c)?
        }
        AtValidateConstraint => CmdEffect::new(ShareUpdateExclusive, Cost::Scan),
        AtSetStatistics | AtSetOptions | AtResetOptions | AtSetRelOptions | AtResetRelOptions => {
            CmdEffect::new(ShareUpdateExclusive, Cost::Metadata)
        }
        AtAttachPartition => {
            let Some(NodeEnum::PartitionCmd(p)) = def else {
                return None;
            };
            let partition = TableRef::from_range_var(p.name.as_ref()?);
            let mut effect = CmdEffect::new(ShareUpdateExclusive, Cost::Metadata);
            effect.extra.push(
                Effect::new(partition, AccessExclusive, Cost::Scan)
                    .with_note("validates the partition constraint"),
            );
            effect
        }
        AtDetachPartition => {
            let concurrent = matches!(def, Some(NodeEnum::PartitionCmd(p)) if p.concurrent);
            let lock = if concurrent {
                ShareUpdateExclusive
            } else {
                AccessExclusive
            };
            CmdEffect::new(lock, Cost::Metadata)
        }
        _ => return None,
    };
    Some(effect)
}

fn add_constraint_effect(c: &Constraint) -> Option<CmdEffect> {
    use LockMode::*;
    let validated = |lock| {
        if c.skip_validation {
            CmdEffect::new(lock, Cost::Metadata)
        } else {
            CmdEffect::new(lock, Cost::Scan)
        }
    };
    let effect = match ConstrType::try_from(c.contype).ok()? {
        ConstrType::ConstrCheck => validated(AccessExclusive),
        ConstrType::ConstrForeign => {
            let mut effect = validated(ShareRowExclusive);
            effect.extra.push(referenced_table_effect(c)?);
            effect
        }
        ConstrType::ConstrUnique | ConstrType::ConstrPrimary => {
            if c.indexname.is_empty() {
                CmdEffect::new(AccessExclusive, Cost::Scan).note("builds an index")
            } else {
                CmdEffect::new(AccessExclusive, Cost::Metadata)
            }
        }
        ConstrType::ConstrExclusion => {
            CmdEffect::new(AccessExclusive, Cost::Scan).note("builds an index")
        }
        _ => return None,
    };
    Some(effect)
}

fn referenced_table_effect(c: &Constraint) -> Option<Effect> {
    let referenced = TableRef::from_range_var(c.pktable.as_ref()?);
    Some(Effect::new(
        referenced,
        LockMode::ShareRowExclusive,
        Cost::Metadata,
    ))
}

fn add_column_effect(column: &ColumnDef) -> CmdEffect {
    let mut effect = CmdEffect::new(LockMode::AccessExclusive, add_column_cost(column));
    for constraint in &column.constraints {
        let Some(NodeEnum::Constraint(c)) = constraint.node.as_ref() else {
            continue;
        };
        let note = match ConstrType::try_from(c.contype) {
            Ok(ConstrType::ConstrCheck | ConstrType::ConstrExclusion) => "validates every row",
            Ok(ConstrType::ConstrUnique | ConstrType::ConstrPrimary) => "builds an index",
            Ok(ConstrType::ConstrForeign) => {
                effect.extra.extend(referenced_table_effect(c));
                "validates the foreign key"
            }
            _ => continue,
        };
        if effect.cost < Cost::Scan {
            effect.cost = Cost::Scan;
            effect.note = Some(note);
        }
    }
    effect
}

/// Cost of the column itself: a rewrite when every row needs a computed value.
fn add_column_cost(column: &ColumnDef) -> Cost {
    let type_name = column
        .type_name
        .as_ref()
        .map_or("", |t| last_name(&t.names));
    if SERIAL_TYPES.contains(&type_name) {
        return Cost::Rewrite;
    }
    let mut defaults = column.raw_default.iter().map(|b| &**b).collect::<Vec<_>>();
    for constraint in &column.constraints {
        let Some(NodeEnum::Constraint(c)) = constraint.node.as_ref() else {
            continue;
        };
        if c.contype == ConstrType::ConstrIdentity as i32
            || c.contype == ConstrType::ConstrGenerated as i32
        {
            return Cost::Rewrite;
        }
        if c.contype == ConstrType::ConstrDefault as i32 {
            defaults.extend(c.raw_expr.as_deref());
        }
    }
    if defaults
        .into_iter()
        .any(|d| classify_default(d) == Volatility::Volatile)
    {
        Cost::Rewrite
    } else {
        Cost::Metadata
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
