//! Shared helpers for the rules about constraints added by ALTER TABLE.

use pg_query::NodeEnum;
use pg_query::protobuf::{
    AlterTableCmd, AlterTableStmt, AlterTableType, ConstrType, Constraint, DropBehavior, Node,
    ObjectType, RangeVar, String as PgString,
};

use super::{added_column, column_constraints, table_alter_cmds};
use crate::model::{TableRef, quote_ident};

/// A constraint added by an ALTER TABLE, either with ADD CONSTRAINT or inline in ADD COLUMN.
pub(crate) struct AddedConstraint<'a> {
    pub constraint: &'a Constraint,
    /// The column it was declared on, for inline constraints.
    pub inline_column: Option<&'a str>,
}

impl AddedConstraint<'_> {
    pub fn is(&self, kind: ConstrType) -> bool {
        self.constraint.contype == kind as i32
    }

    /// Column names the constraint covers (the inline column, or the listed keys).
    pub fn columns(&self) -> Vec<String> {
        if let Some(column) = self.inline_column {
            return vec![column.to_string()];
        }
        let keys = if self.is(ConstrType::ConstrForeign) {
            &self.constraint.fk_attrs
        } else {
            &self.constraint.keys
        };
        keys.iter()
            .filter_map(|k| match k.node.as_ref() {
                Some(NodeEnum::String(s)) => Some(s.sval.clone()),
                _ => None,
            })
            .collect()
    }

    /// The constraint's name, or the one PostgreSQL would generate.
    pub fn name(&self, table: &TableRef) -> String {
        if !self.constraint.conname.is_empty() {
            return self.constraint.conname.clone();
        }
        let columns = self.columns();
        let first = columns.first().map(String::as_str).unwrap_or("col");
        match ConstrType::try_from(self.constraint.contype) {
            Ok(ConstrType::ConstrPrimary) => format!("{}_pkey", table.name),
            Ok(ConstrType::ConstrUnique) => format!("{}_{}_key", table.name, columns.join("_")),
            Ok(ConstrType::ConstrForeign) => format!("{}_{first}_fkey", table.name),
            _ => format!("{}_check", table.name),
        }
    }
}

pub(crate) fn added_constraints(node: &NodeEnum) -> Vec<AddedConstraint<'_>> {
    let mut found = Vec::new();
    for cmd in table_alter_cmds(node) {
        if cmd.subtype == AlterTableType::AtAddConstraint as i32 {
            if let Some(NodeEnum::Constraint(c)) = cmd.def.as_ref().and_then(|d| d.node.as_ref()) {
                found.push(AddedConstraint {
                    constraint: c,
                    inline_column: None,
                });
            }
        } else if let Some(column) = added_column(cmd) {
            for c in column_constraints(column) {
                found.push(AddedConstraint {
                    constraint: c,
                    inline_column: Some(&column.colname),
                });
            }
        }
    }
    found
}

pub(crate) fn relation(node: &NodeEnum) -> Option<&RangeVar> {
    match node {
        NodeEnum::AlterTableStmt(stmt) => stmt.relation.as_ref(),
        _ => None,
    }
}

/// `ALTER TABLE t ADD CONSTRAINT c ... NOT VALID;` followed by `VALIDATE CONSTRAINT c;`.
pub(crate) fn not_valid_then_validate(relation: &RangeVar, added: &AddedConstraint) -> String {
    let table = TableRef::from_range_var(relation);
    let name = added.name(&table);
    let mut constraint = added.constraint.clone();
    constraint.conname = name.clone();
    constraint.skip_validation = true;
    constraint.initially_valid = false;
    if constraint.contype == ConstrType::ConstrForeign as i32 && constraint.fk_attrs.is_empty() {
        constraint.fk_attrs = added
            .columns()
            .into_iter()
            .map(|sval| Node {
                node: Some(NodeEnum::String(PgString { sval })),
            })
            .collect();
    }
    let stmt = AlterTableStmt {
        relation: Some(relation.clone()),
        cmds: vec![Node {
            node: Some(NodeEnum::AlterTableCmd(Box::new(AlterTableCmd {
                subtype: AlterTableType::AtAddConstraint as i32,
                def: Some(Box::new(Node {
                    node: Some(NodeEnum::Constraint(Box::new(constraint))),
                })),
                // The C deparser asserts on enum value 0 (Undefined).
                behavior: DropBehavior::DropRestrict as i32,
                ..AlterTableCmd::default()
            }))),
        }],
        objtype: ObjectType::ObjectTable as i32,
        ..AlterTableStmt::default()
    };
    let quoted = quote_ident(&name);
    let add = NodeEnum::AlterTableStmt(stmt)
        .deparse()
        .unwrap_or_else(|_| format!("ALTER TABLE {table} ADD CONSTRAINT {quoted} ... NOT VALID"));
    let sql = format!("{add};\nALTER TABLE {table} VALIDATE CONSTRAINT {quoted};");
    if added.inline_column.is_some() {
        format!("Add the column without the constraint, then:\n{sql}")
    } else {
        sql
    }
}
