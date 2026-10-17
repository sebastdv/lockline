//! A minimal schema built from the statements seen so far in the run.
//!
//! It only knows what the migrations themselves say: column types, `CHECK (col IS NOT NULL)`
//! constraints and which table each named index belongs to. lockline uses it to recognize
//! safe type widenings, `SET NOT NULL` backed by a validated check, and the table behind an
//! index name.

use std::collections::HashMap;

use pg_query::NodeEnum;
use pg_query::protobuf::{
    AlterTableStmt, AlterTableType, ConstrType, Constraint, DropStmt, Node, NullTestType,
    ObjectType, RenameStmt, TypeName, a_const,
};

use crate::model::TableRef;
use crate::volatility::last_name;

fn column_constraints(column: &pg_query::protobuf::ColumnDef) -> impl Iterator<Item = &Constraint> {
    column
        .constraints
        .iter()
        .filter_map(|c| match c.node.as_ref() {
            Some(NodeEnum::Constraint(c)) => Some(&**c),
            _ => None,
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSpec {
    /// Last component of the type name as the parser normalizes it (`varchar`, `int4`…).
    pub name: String,
    /// Integer type modifiers, such as the length of `varchar(100)`.
    pub mods: Vec<i64>,
}

impl TypeSpec {
    pub fn from_type_name(t: &TypeName) -> TypeSpec {
        let mods = t
            .typmods
            .iter()
            .filter_map(|m| match m.node.as_ref() {
                Some(NodeEnum::AConst(c)) => match &c.val {
                    Some(a_const::Val::Ival(i)) => Some(i64::from(i.ival)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        TypeSpec {
            name: last_name(&t.names).to_string(),
            mods,
        }
    }
}

/// Type changes PostgreSQL applies without rewriting the table.
pub fn is_safe_widening(old: &TypeSpec, new: &TypeSpec) -> bool {
    if old == new {
        return true;
    }
    match (old.name.as_str(), new.name.as_str()) {
        ("varchar", "varchar") | ("varbit", "varbit") => {
            match (old.mods.first(), new.mods.first()) {
                (_, None) => true,
                (Some(o), Some(n)) => n >= o,
                (None, Some(_)) => false,
            }
        }
        ("varchar", "text") => true,
        ("numeric", "numeric") => match (old.mods.as_slice(), new.mods.as_slice()) {
            ([p, s], [p2, s2]) => p2 >= p && s2 == s,
            ([p], [p2]) => p2 >= p,
            _ => false,
        },
        _ => false,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    tables: HashMap<TableRef, TableInfo>,
    /// Index (in its table's schema) -> table.
    indexes: HashMap<TableRef, TableRef>,
}

#[derive(Debug, Clone, Default)]
struct TableInfo {
    columns: HashMap<String, TypeSpec>,
    not_null_checks: Vec<NotNullCheck>,
    /// Foreign key constraint name -> referenced table.
    foreign_keys: HashMap<String, TableRef>,
}

#[derive(Debug, Clone)]
struct NotNullCheck {
    /// Empty for unnamed checks.
    name: String,
    column: String,
    validated: bool,
}

impl Schema {
    pub fn column_type(&self, table: &TableRef, column: &str) -> Option<&TypeSpec> {
        self.tables.get(table)?.columns.get(column)
    }

    pub fn has_validated_not_null_check(&self, table: &TableRef, column: &str) -> bool {
        self.tables.get(table).is_some_and(|t| {
            t.not_null_checks
                .iter()
                .any(|c| c.column == column && c.validated)
        })
    }

    /// The table a foreign key constraint of `table` references.
    pub fn foreign_key_target(&self, table: &TableRef, constraint: &str) -> Option<&TableRef> {
        self.tables.get(table)?.foreign_keys.get(constraint)
    }

    pub fn index_table(&self, index: &TableRef) -> Option<&TableRef> {
        self.indexes.get(index)
    }

    pub fn observe(&mut self, node: &NodeEnum) {
        match node {
            NodeEnum::CreateStmt(stmt) => {
                let Some(relation) = &stmt.relation else {
                    return;
                };
                let table = TableRef::from_range_var(relation);
                if stmt.if_not_exists && self.tables.contains_key(&table) {
                    return;
                }
                let mut info = TableInfo::default();
                for elt in &stmt.table_elts {
                    match elt.node.as_ref() {
                        Some(NodeEnum::ColumnDef(column)) => {
                            if let Some(t) = &column.type_name {
                                info.columns
                                    .insert(column.colname.clone(), TypeSpec::from_type_name(t));
                            }
                            for c in column_constraints(column) {
                                info.add_foreign_key(c, &table.name, Some(&column.colname));
                            }
                        }
                        Some(NodeEnum::Constraint(c)) => {
                            info.add_check(c);
                            info.add_foreign_key(c, &table.name, None);
                        }
                        _ => {}
                    }
                }
                self.tables.insert(table, info);
            }
            NodeEnum::AlterTableStmt(stmt) => self.observe_alter_table(stmt),
            NodeEnum::RenameStmt(stmt) => self.observe_rename(stmt),
            NodeEnum::DropStmt(stmt) => self.observe_drop(stmt),
            NodeEnum::IndexStmt(stmt) => {
                if let Some(relation) = &stmt.relation
                    && !stmt.idxname.is_empty()
                {
                    let table = TableRef::from_range_var(relation);
                    let index = TableRef {
                        schema: table.schema.clone(),
                        name: stmt.idxname.clone(),
                    };
                    self.indexes.insert(index, table);
                }
            }
            _ => {}
        }
    }

    fn observe_alter_table(&mut self, stmt: &AlterTableStmt) {
        if stmt.objtype != ObjectType::ObjectTable as i32 {
            return;
        }
        let Some(relation) = &stmt.relation else {
            return;
        };
        let table = TableRef::from_range_var(relation);
        let info = self.tables.entry(table.clone()).or_default();
        for cmd in &stmt.cmds {
            let Some(NodeEnum::AlterTableCmd(cmd)) = cmd.node.as_ref() else {
                continue;
            };
            let def = cmd.def.as_ref().and_then(|d| d.node.as_ref());
            match AlterTableType::try_from(cmd.subtype) {
                Ok(AlterTableType::AtAddColumn) | Ok(AlterTableType::AtAlterColumnType) => {
                    if let Some(NodeEnum::ColumnDef(column)) = def
                        && let Some(t) = &column.type_name
                    {
                        let name = if cmd.name.is_empty() {
                            column.colname.clone()
                        } else {
                            cmd.name.clone()
                        };
                        info.columns.insert(name, TypeSpec::from_type_name(t));
                    }
                    if let Some(NodeEnum::ColumnDef(column)) = def {
                        for c in column_constraints(column) {
                            info.add_foreign_key(c, &table.name, Some(&column.colname));
                        }
                    }
                }
                Ok(AlterTableType::AtDropColumn) => {
                    info.columns.remove(&cmd.name);
                    info.not_null_checks.retain(|c| c.column != cmd.name);
                }
                Ok(AlterTableType::AtAddConstraint) => {
                    if let Some(NodeEnum::Constraint(c)) = def {
                        info.add_check(c);
                        info.add_foreign_key(c, &table.name, None);
                    }
                }
                Ok(AlterTableType::AtValidateConstraint) => {
                    for check in &mut info.not_null_checks {
                        if check.name == cmd.name {
                            check.validated = true;
                        }
                    }
                }
                Ok(AlterTableType::AtDropConstraint) => {
                    info.not_null_checks.retain(|c| c.name != cmd.name);
                    info.foreign_keys.remove(&cmd.name);
                }
                _ => {}
            }
        }
    }

    fn observe_rename(&mut self, stmt: &RenameStmt) {
        let Some(relation) = &stmt.relation else {
            return;
        };
        let table = TableRef::from_range_var(relation);
        if stmt.rename_type == ObjectType::ObjectTable as i32 {
            let renamed = TableRef {
                schema: table.schema.clone(),
                name: stmt.newname.clone(),
            };
            if let Some(info) = self.tables.remove(&table) {
                self.tables.insert(renamed.clone(), info);
            }
            for owner in self.indexes.values_mut() {
                if *owner == table {
                    *owner = renamed.clone();
                }
            }
        } else if stmt.rename_type == ObjectType::ObjectColumn as i32
            && let Some(info) = self.tables.get_mut(&table)
        {
            if let Some(t) = info.columns.remove(&stmt.subname) {
                info.columns.insert(stmt.newname.clone(), t);
            }
            for check in &mut info.not_null_checks {
                if check.column == stmt.subname {
                    check.column = stmt.newname.clone();
                }
            }
        }
    }

    fn observe_drop(&mut self, stmt: &DropStmt) {
        let is_table = stmt.remove_type == ObjectType::ObjectTable as i32;
        let is_index = stmt.remove_type == ObjectType::ObjectIndex as i32;
        for object in &stmt.objects {
            let Some(name) = qualified_name(object) else {
                continue;
            };
            if is_table {
                self.tables.remove(&name);
                self.indexes.retain(|_, owner| *owner != name);
            } else if is_index {
                self.indexes.remove(&name);
            }
        }
    }
}

impl TableInfo {
    /// Records a foreign key under its name, or the one PostgreSQL generates
    /// (`{table}_{columns}_fkey`).
    fn add_foreign_key(&mut self, c: &Constraint, table: &str, inline_column: Option<&str>) {
        if c.contype != ConstrType::ConstrForeign as i32 {
            return;
        }
        let Some(referenced) = &c.pktable else {
            return;
        };
        let name = if c.conname.is_empty() {
            let columns: Vec<&str> = match inline_column {
                Some(column) => vec![column],
                None => c
                    .fk_attrs
                    .iter()
                    .filter_map(|a| match a.node.as_ref() {
                        Some(NodeEnum::String(s)) => Some(s.sval.as_str()),
                        _ => None,
                    })
                    .collect(),
            };
            format!("{table}_{}_fkey", columns.join("_"))
        } else {
            c.conname.clone()
        };
        self.foreign_keys
            .insert(name, TableRef::from_range_var(referenced));
    }

    /// Records `CHECK (col IS NOT NULL)`; other constraints are ignored.
    fn add_check(&mut self, c: &Constraint) {
        if c.contype != ConstrType::ConstrCheck as i32 {
            return;
        }
        let Some(NodeEnum::NullTest(test)) = c.raw_expr.as_ref().and_then(|e| e.node.as_ref())
        else {
            return;
        };
        if test.nulltesttype != NullTestType::IsNotNull as i32 {
            return;
        }
        let Some(NodeEnum::ColumnRef(column)) = test.arg.as_ref().and_then(|a| a.node.as_ref())
        else {
            return;
        };
        self.not_null_checks.push(NotNullCheck {
            name: c.conname.clone(),
            column: last_name(&column.fields).to_string(),
            validated: !c.skip_validation,
        });
    }
}

/// `schema.name` from a `List` of `String` nodes, as used by DROP and COMMENT.
pub(crate) fn qualified_name(node: &Node) -> Option<TableRef> {
    let Some(NodeEnum::List(list)) = node.node.as_ref() else {
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
        [.., schema, name] => Some(TableRef {
            schema: Some(schema.to_string()),
            name: name.to_string(),
        }),
        [name] => Some(TableRef {
            schema: None,
            name: name.to_string(),
        }),
        [] => None,
    }
}
