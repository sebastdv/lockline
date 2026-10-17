//! Maps a parsed statement to the locks it takes and the work it does on each table.
//!
//! Anything outside the catalog is reported as `NotAnalyzed` rather than guessed.
//! `effects[0]` is always the relation the statement changes; other tables follow.

use pg_query::NodeEnum;
use pg_query::protobuf::{
    AlterTableCmd, AlterTableStmt, AlterTableType, ColumnDef, ConstrType, Constraint, CreateStmt,
    DefElem, DropStmt, Node, ObjectType, RangeVar, ReindexObjectType, ReindexStmt, a_const,
};

use crate::locks::LockMode;
use crate::model::{Analysis, Cost, Effect, TableRef};
use crate::schema::{Schema, TypeSpec, is_safe_widening, qualified_name};
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
        NodeEnum::CreateStmt(stmt) => analyze_create_table(stmt),
        NodeEnum::ViewStmt(stmt) if stmt.replace => match &stmt.view {
            Some(view) => locks(vec![
                Effect::new(
                    TableRef::from_range_var(view),
                    LockMode::AccessExclusive,
                    Cost::Metadata,
                )
                .with_note("if the view already exists"),
            ]),
            None => Analysis::NotAnalyzed,
        },
        NodeEnum::DropStmt(stmt) => analyze_drop(stmt, schema),
        NodeEnum::ReindexStmt(stmt) => analyze_reindex(stmt, schema),
        NodeEnum::VacuumStmt(stmt) => {
            let full = stmt.is_vacuumcmd && has_option(&stmt.options, "full");
            let (lock, cost) = if full {
                (LockMode::AccessExclusive, Cost::Rewrite)
            } else {
                (LockMode::ShareUpdateExclusive, Cost::Scan)
            };
            let tables = stmt.rels.iter().filter_map(|r| match r.node.as_ref() {
                Some(NodeEnum::VacuumRelation(v)) => v.relation.as_ref(),
                _ => None,
            });
            locks(
                tables
                    .map(|t| Effect::new(TableRef::from_range_var(t), lock, cost))
                    .collect(),
            )
        }
        NodeEnum::ClusterStmt(stmt) => match &stmt.relation {
            Some(relation) => locks(vec![Effect::new(
                TableRef::from_range_var(relation),
                LockMode::AccessExclusive,
                Cost::Rewrite,
            )]),
            None => Analysis::NotAnalyzed,
        },
        NodeEnum::RefreshMatViewStmt(stmt) => {
            // CONCURRENTLY keeps the same file and applies the difference (verified on
            // PostgreSQL); the plain refresh swaps in a new file.
            let (lock, cost, note) = if stmt.concurrent {
                (
                    LockMode::Exclusive,
                    Cost::Scan,
                    "computes the new contents and applies the difference",
                )
            } else {
                (
                    LockMode::AccessExclusive,
                    Cost::Rewrite,
                    "rebuilds the materialized view",
                )
            };
            relation_effect(stmt.relation.as_ref(), lock, cost, Some(note))
        }
        NodeEnum::CreateTrigStmt(stmt) => relation_effect(
            stmt.relation.as_ref(),
            LockMode::ShareRowExclusive,
            Cost::Metadata,
            None,
        ),
        NodeEnum::TruncateStmt(stmt) => locks(
            stmt.relations
                .iter()
                .filter_map(|r| match r.node.as_ref() {
                    Some(NodeEnum::RangeVar(rv)) => Some(Effect::new(
                        TableRef::from_range_var(rv),
                        LockMode::AccessExclusive,
                        Cost::Metadata,
                    )),
                    _ => None,
                })
                .collect(),
        ),
        NodeEnum::InsertStmt(stmt) => relation_effect(
            stmt.relation.as_ref(),
            LockMode::RowExclusive,
            Cost::Unknown,
            None,
        ),
        NodeEnum::UpdateStmt(stmt) => relation_effect(
            stmt.relation.as_ref(),
            LockMode::RowExclusive,
            Cost::Unknown,
            None,
        ),
        NodeEnum::DeleteStmt(stmt) => relation_effect(
            stmt.relation.as_ref(),
            LockMode::RowExclusive,
            Cost::Unknown,
            None,
        ),
        NodeEnum::CommentStmt(stmt) => {
            let target = stmt.object.as_deref().and_then(|object| {
                if stmt.objtype == ObjectType::ObjectTable as i32 {
                    qualified_name(object)
                } else if stmt.objtype == ObjectType::ObjectColumn as i32 {
                    column_owner(object)
                } else {
                    None
                }
            });
            match target {
                Some(table) => locks(vec![Effect::new(
                    table,
                    LockMode::ShareUpdateExclusive,
                    Cost::Metadata,
                )]),
                None => Analysis::NotAnalyzed,
            }
        }
        NodeEnum::RenameStmt(stmt) => {
            let renames_table_part = [
                ObjectType::ObjectTable,
                ObjectType::ObjectColumn,
                ObjectType::ObjectTabconstraint,
            ]
            .iter()
            .any(|t| *t as i32 == stmt.rename_type);
            if renames_table_part {
                relation_effect(
                    stmt.relation.as_ref(),
                    LockMode::AccessExclusive,
                    Cost::Metadata,
                    None,
                )
            } else {
                Analysis::NotAnalyzed
            }
        }
        NodeEnum::TransactionStmt(_)
        | NodeEnum::VariableSetStmt(_)
        | NodeEnum::CreateFunctionStmt(_)
        | NodeEnum::ViewStmt(_)
        | NodeEnum::CreateEnumStmt(_) => Analysis::NoTableLock,
        _ => Analysis::NotAnalyzed,
    }
}

const UNKNOWN_INDEX_TABLE: &str = "on the index's table (not seen in this run)";

/// `Locks` with at least one effect; an empty list means the statement names no table.
fn locks(effects: Vec<Effect>) -> Analysis {
    if effects.is_empty() {
        Analysis::NotAnalyzed
    } else {
        Analysis::Locks(effects)
    }
}

fn relation_effect(
    relation: Option<&RangeVar>,
    lock: LockMode,
    cost: Cost,
    note: Option<&str>,
) -> Analysis {
    let Some(relation) = relation else {
        return Analysis::NotAnalyzed;
    };
    let effect = Effect::new(TableRef::from_range_var(relation), lock, cost);
    locks(vec![match note {
        Some(note) => effect.with_note(note),
        None => effect,
    }])
}

/// A new table takes no lock on existing tables, except on the ones it attaches to.
fn analyze_create_table(stmt: &CreateStmt) -> Analysis {
    let mut effects = Vec::new();
    if stmt.partbound.is_some() {
        for parent in range_vars(&stmt.inh_relations) {
            effects.push(Effect::new(
                parent,
                LockMode::AccessExclusive,
                Cost::Metadata,
            ));
        }
    } else {
        for parent in range_vars(&stmt.inh_relations) {
            effects.push(Effect::new(
                parent,
                LockMode::ShareUpdateExclusive,
                Cost::Metadata,
            ));
        }
    }
    for elt in stmt.table_elts.iter().chain(&stmt.constraints) {
        let constraints: Vec<&Constraint> = match elt.node.as_ref() {
            Some(NodeEnum::Constraint(c)) => vec![c],
            Some(NodeEnum::ColumnDef(column)) => column
                .constraints
                .iter()
                .filter_map(|c| match c.node.as_ref() {
                    Some(NodeEnum::Constraint(c)) => Some(&**c),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        for c in constraints {
            if c.contype == ConstrType::ConstrForeign as i32
                && let Some(effect) = referenced_table_effect(c)
                && !effects.iter().any(|e: &Effect| e.table == effect.table)
            {
                effects.push(effect);
            }
        }
    }
    if effects.is_empty() {
        Analysis::NoTableLock
    } else {
        Analysis::Locks(effects)
    }
}

fn range_vars(nodes: &[Node]) -> impl Iterator<Item = TableRef> + '_ {
    nodes.iter().filter_map(|n| match n.node.as_ref() {
        Some(NodeEnum::RangeVar(rv)) => Some(TableRef::from_range_var(rv)),
        _ => None,
    })
}

fn analyze_drop(stmt: &DropStmt, schema: &Schema) -> Analysis {
    let names = stmt.objects.iter().filter_map(qualified_name);
    if stmt.remove_type == ObjectType::ObjectTable as i32 {
        let dropped: Vec<TableRef> = names.collect();
        let mut effects: Vec<Effect> = dropped
            .iter()
            .map(|t| Effect::new(t.clone(), LockMode::AccessExclusive, Cost::Metadata))
            .collect();
        // The dropped tables' foreign keys lose their triggers on the referenced tables,
        // which takes ACCESS EXCLUSIVE there (verified on PostgreSQL).
        for table in &dropped {
            for referenced in schema.referenced_tables(table, None) {
                if !effects.iter().any(|e| e.table == *referenced) {
                    effects.push(Effect::new(
                        referenced.clone(),
                        LockMode::AccessExclusive,
                        Cost::Metadata,
                    ));
                }
            }
        }
        locks(effects)
    } else if stmt.remove_type == ObjectType::ObjectIndex as i32 {
        let lock = if stmt.concurrent {
            LockMode::ShareUpdateExclusive
        } else {
            LockMode::AccessExclusive
        };
        locks(
            names
                .map(|index| index_table_effect(&index, schema, lock, Cost::Metadata, None))
                .collect(),
        )
    } else {
        Analysis::NotAnalyzed
    }
}

fn analyze_reindex(stmt: &ReindexStmt, schema: &Schema) -> Analysis {
    let concurrent = has_option(&stmt.params, "concurrently");
    let lock = if concurrent {
        LockMode::ShareUpdateExclusive
    } else {
        LockMode::Share
    };
    let Some(relation) = &stmt.relation else {
        return Analysis::NotAnalyzed;
    };
    let target = TableRef::from_range_var(relation);
    match ReindexObjectType::try_from(stmt.kind) {
        Ok(ReindexObjectType::ReindexObjectTable) => locks(vec![
            Effect::new(target, lock, Cost::Scan).with_note(if concurrent {
                "rebuilds the table's indexes"
            } else {
                "rebuilds the table's indexes; ACCESS EXCLUSIVE on each index blocks queries that use it"
            }),
        ]),
        Ok(ReindexObjectType::ReindexObjectIndex) => locks(vec![index_table_effect(
            &target,
            schema,
            lock,
            Cost::Scan,
            Some(if concurrent {
                "rebuilds the index"
            } else {
                "rebuilds the index; ACCESS EXCLUSIVE on the index blocks queries that use it"
            }),
        )]),
        _ => Analysis::NotAnalyzed,
    }
}

/// The lock lands on the index's table; when the run never created that index, the effect
/// names the index itself and says so.
fn index_table_effect(
    index: &TableRef,
    schema: &Schema,
    lock: LockMode,
    cost: Cost,
    note: Option<&str>,
) -> Effect {
    match schema.index_table(index) {
        Some(table) => {
            let effect = Effect::new(table.clone(), lock, cost);
            match note {
                Some(note) => effect.with_note(note),
                None => effect,
            }
        }
        None => {
            let note = match note {
                Some(note) => format!("{note}; {UNKNOWN_INDEX_TABLE}"),
                None => UNKNOWN_INDEX_TABLE.to_string(),
            };
            Effect::new(index.clone(), lock, cost).with_note(&note)
        }
    }
}

/// The table of a `COMMENT ON COLUMN` target (`[schema.]table.column`).
fn column_owner(object: &Node) -> Option<TableRef> {
    let NodeEnum::List(list) = object.node.as_ref()? else {
        return None;
    };
    let names: Vec<&str> = list
        .items
        .iter()
        .filter_map(|n| match n.node.as_ref() {
            Some(NodeEnum::String(s)) => Some(s.sval.as_str()),
            _ => None,
        })
        .collect();
    match names.as_slice() {
        [.., schema, table, _column] => Some(TableRef {
            schema: Some(schema.to_string()),
            name: table.to_string(),
        }),
        [table, _column] => Some(TableRef {
            schema: None,
            name: table.to_string(),
        }),
        _ => None,
    }
}

/// Whether a `(option [value])` list turns `name` on, with `defGetBoolean` semantics.
fn has_option(options: &[Node], name: &str) -> bool {
    options.iter().any(|o| {
        matches!(o.node.as_ref(), Some(NodeEnum::DefElem(d)) if d.defname == name && def_elem_bool(d))
    })
}

/// `defGetBoolean`: no value means true; otherwise true/on/yes/1 or false/off/no/0.
pub(crate) fn def_elem_bool(d: &DefElem) -> bool {
    let Some(arg) = d.arg.as_deref().and_then(|a| a.node.as_ref()) else {
        return true;
    };
    match arg {
        NodeEnum::Boolean(b) => b.boolval,
        NodeEnum::Integer(i) => i.ival != 0,
        NodeEnum::String(s) => !matches!(
            s.sval.to_ascii_lowercase().as_str(),
            "false" | "off" | "no" | "0"
        ),
        NodeEnum::AConst(c) => match &c.val {
            Some(a_const::Val::Boolval(b)) => b.boolval,
            Some(a_const::Val::Ival(i)) => i.ival != 0,
            Some(a_const::Val::Sval(s)) => !matches!(
                s.sval.to_ascii_lowercase().as_str(),
                "false" | "off" | "no" | "0"
            ),
            _ => true,
        },
        _ => true,
    }
}

/// Statements PostgreSQL refuses to run inside a transaction block, with the label to show.
pub fn forbidden_in_transaction(node: &NodeEnum) -> Option<&'static str> {
    match node {
        NodeEnum::IndexStmt(stmt) if stmt.concurrent => Some("CREATE INDEX CONCURRENTLY"),
        NodeEnum::DropStmt(stmt) if stmt.concurrent => Some("DROP INDEX CONCURRENTLY"),
        NodeEnum::ReindexStmt(stmt) if has_option(&stmt.params, "concurrently") => {
            Some("REINDEX CONCURRENTLY")
        }
        NodeEnum::AlterTableStmt(stmt) if alter_cmds(stmt).any(is_detach_concurrently) => {
            Some("DETACH PARTITION CONCURRENTLY")
        }
        NodeEnum::VacuumStmt(stmt) if stmt.is_vacuumcmd => Some("VACUUM"),
        _ => None,
    }
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
        AtDropNotNull | AtColumnDefault => CmdEffect::new(AccessExclusive, Cost::Metadata),
        // Dropping a foreign key (or the column it is on) also removes its triggers on the
        // referenced table: ACCESS EXCLUSIVE there too (verified on PostgreSQL).
        AtDropColumn | AtDropConstraint => {
            let referenced = if cmd.subtype == AtDropColumn as i32 {
                schema.referenced_tables(table, Some(&cmd.name))
            } else {
                schema
                    .foreign_key_target(table, &cmd.name)
                    .into_iter()
                    .collect()
            };
            let mut effect = CmdEffect::new(AccessExclusive, Cost::Metadata);
            for other in referenced.into_iter().filter(|t| *t != table) {
                effect
                    .extra
                    .push(Effect::new(other.clone(), AccessExclusive, Cost::Metadata));
            }
            effect
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
        AtValidateConstraint => {
            let mut effect = CmdEffect::new(ShareUpdateExclusive, Cost::Scan);
            // Validating a foreign key reads the referenced table (verified on PostgreSQL).
            if let Some(referenced) = schema.foreign_key_target(table, &cmd.name) {
                effect
                    .extra
                    .push(Effect::new(referenced.clone(), RowShare, Cost::Metadata));
            }
            effect
        }
        AtSetStatistics | AtSetOptions | AtResetOptions | AtClusterOn | AtDropCluster => {
            CmdEffect::new(ShareUpdateExclusive, Cost::Metadata)
        }
        // Most storage parameters take SHARE UPDATE EXCLUSIVE; user_catalog_table takes
        // ACCESS EXCLUSIVE (verified on PostgreSQL 14-17). The strongest option wins.
        AtSetRelOptions | AtResetRelOptions => {
            let lock = if names_option(def, "user_catalog_table") {
                AccessExclusive
            } else {
                ShareUpdateExclusive
            };
            CmdEffect::new(lock, Cost::Metadata)
        }
        // Verified on PostgreSQL 14-17.
        AtSetStorage | AtSetCompression | AtEnableRule | AtDisableRule => {
            CmdEffect::new(AccessExclusive, Cost::Metadata)
        }
        AtAddIdentity | AtSetIdentity | AtDropIdentity | AtReplicaIdentity | AtChangeOwner
        | AtAlterConstraint | AtEnableRowSecurity | AtDisableRowSecurity | AtForceRowSecurity
        | AtNoForceRowSecurity => CmdEffect::new(AccessExclusive, Cost::Metadata),
        AtEnableTrig | AtEnableAlwaysTrig | AtEnableReplicaTrig | AtDisableTrig
        | AtEnableTrigAll | AtDisableTrigAll | AtEnableTrigUser | AtDisableTrigUser => {
            CmdEffect::new(ShareRowExclusive, Cost::Metadata)
        }
        AtSetLogged | AtSetUnLogged => {
            CmdEffect::new(AccessExclusive, Cost::Rewrite).note("rewrites the table")
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
            let Some(NodeEnum::PartitionCmd(p)) = def else {
                return None;
            };
            if p.concurrent {
                // The second of its two transactions takes ACCESS EXCLUSIVE on the partition
                // (verified on PostgreSQL).
                let mut effect = CmdEffect::new(ShareUpdateExclusive, Cost::Metadata);
                effect.extra.push(
                    Effect::new(
                        TableRef::from_range_var(p.name.as_ref()?),
                        AccessExclusive,
                        Cost::Metadata,
                    )
                    .with_note("in its second transaction"),
                );
                effect
            } else {
                let mut effect = CmdEffect::new(AccessExclusive, Cost::Metadata);
                // The partition is locked too (verified on PostgreSQL).
                effect.extra.push(Effect::new(
                    TableRef::from_range_var(p.name.as_ref()?),
                    AccessExclusive,
                    Cost::Metadata,
                ));
                effect
            }
        }
        _ => return None,
    };
    Some(effect)
}

/// Whether a `SET (...)` / `RESET (...)` option list mentions `name`.
fn names_option(def: Option<&NodeEnum>, name: &str) -> bool {
    let Some(NodeEnum::List(list)) = def else {
        return false;
    };
    list.items
        .iter()
        .any(|item| matches!(item.node.as_ref(), Some(NodeEnum::DefElem(d)) if d.defname == name))
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
            } else if c.contype == ConstrType::ConstrPrimary as i32 {
                // PRIMARY KEY implies NOT NULL: PostgreSQL scans for NULLs unless the columns
                // are already NOT NULL, which lockline cannot know.
                CmdEffect::new(AccessExclusive, Cost::Scan)
                    .note("scans for NULLs unless the columns are already NOT NULL")
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

/// Why adding a column has to write every existing row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RewriteCause {
    VolatileDefault,
    Serial,
    Identity,
    Generated,
}

pub(crate) fn rewrite_cause(column: &ColumnDef) -> Option<RewriteCause> {
    let type_name = column
        .type_name
        .as_ref()
        .map_or("", |t| last_name(&t.names));
    if SERIAL_TYPES.contains(&type_name) {
        return Some(RewriteCause::Serial);
    }
    let mut defaults = column.raw_default.iter().map(|b| &**b).collect::<Vec<_>>();
    for constraint in &column.constraints {
        let Some(NodeEnum::Constraint(c)) = constraint.node.as_ref() else {
            continue;
        };
        if c.contype == ConstrType::ConstrIdentity as i32 {
            return Some(RewriteCause::Identity);
        }
        if c.contype == ConstrType::ConstrGenerated as i32 {
            return Some(RewriteCause::Generated);
        }
        if c.contype == ConstrType::ConstrDefault as i32 {
            defaults.extend(c.raw_expr.as_deref());
        }
    }
    defaults
        .into_iter()
        .any(|d| classify_default(d) == Volatility::Volatile)
        .then_some(RewriteCause::VolatileDefault)
}

/// Cost of the column itself: a rewrite when every row needs a computed value.
pub(crate) fn add_column_cost(column: &ColumnDef) -> Cost {
    if rewrite_cause(column).is_some() {
        Cost::Rewrite
    } else {
        Cost::Metadata
    }
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
