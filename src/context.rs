//! What the analyzer knows while walking one migration unit.

use std::collections::HashSet;

use pg_query::NodeEnum;
use pg_query::protobuf::{
    Node, ObjectType, RangeVar, TransactionStmtKind, VariableSetKind, VariableSetStmt, a_const,
};

use crate::analyze::TransactionMode;
use crate::model::TableRef;
use crate::schema::qualified_name;

#[derive(Debug, Clone)]
pub struct UnitContext {
    /// Tables created earlier in this unit: they have no rows and no traffic yet.
    pub created_tables: HashSet<TableRef>,
    pub in_transaction: bool,
    lock_timeout_session: bool,
    /// `SET LOCAL lock_timeout`: only until the current transaction ends.
    lock_timeout_local: bool,
}

impl UnitContext {
    pub fn new(mode: TransactionMode) -> Self {
        UnitContext {
            created_tables: HashSet::new(),
            in_transaction: mode == TransactionMode::PerUnit,
            lock_timeout_session: false,
            lock_timeout_local: false,
        }
    }

    /// A non-zero `lock_timeout` is in effect for the next statement.
    pub fn lock_timeout_set(&self) -> bool {
        self.lock_timeout_session || (self.in_transaction && self.lock_timeout_local)
    }

    /// Updates the state after a statement has been checked by the rules.
    pub fn observe(&mut self, node: &NodeEnum) {
        match node {
            // `IF NOT EXISTS` may be a no-op on an existing table with rows and traffic,
            // so it does not make the table "new".
            NodeEnum::CreateStmt(stmt) if !stmt.if_not_exists => {
                self.mark_created(stmt.relation.as_ref());
            }
            NodeEnum::CreateTableAsStmt(stmt) if !stmt.if_not_exists => {
                self.mark_created(stmt.into.as_ref().and_then(|i| i.rel.as_ref()));
            }
            // A new table keeps being new under its new name; a dropped one is gone.
            NodeEnum::RenameStmt(stmt) if stmt.rename_type == ObjectType::ObjectTable as i32 => {
                if let Some(relation) = &stmt.relation {
                    let old = TableRef::from_range_var(relation);
                    if self.created_tables.remove(&old) {
                        self.created_tables.insert(TableRef {
                            schema: old.schema,
                            name: stmt.newname.clone(),
                        });
                    }
                }
            }
            NodeEnum::DropStmt(stmt) if stmt.remove_type == ObjectType::ObjectTable as i32 => {
                for name in stmt.objects.iter().filter_map(qualified_name) {
                    self.created_tables.remove(&name);
                }
            }
            NodeEnum::SelectStmt(stmt) => {
                self.mark_created(stmt.into_clause.as_ref().and_then(|i| i.rel.as_ref()));
            }
            NodeEnum::TransactionStmt(stmt) => {
                let kind = stmt.kind;
                if kind == TransactionStmtKind::TransStmtBegin as i32
                    || kind == TransactionStmtKind::TransStmtStart as i32
                {
                    self.in_transaction = true;
                } else if ends_transaction(node) {
                    // `AND CHAIN` starts a new transaction right away.
                    self.in_transaction = stmt.chain;
                    self.lock_timeout_local = false;
                }
            }
            NodeEnum::VariableSetStmt(stmt) => self.observe_set(stmt),
            _ => {}
        }
    }

    fn observe_set(&mut self, stmt: &VariableSetStmt) {
        if stmt.kind == VariableSetKind::VarResetAll as i32 {
            self.lock_timeout_session = false;
            return;
        }
        if stmt.name != "lock_timeout" {
            return;
        }
        let enabled = stmt.kind == VariableSetKind::VarSetValue as i32
            && !stmt.args.first().is_some_and(is_zero);
        if stmt.is_local {
            // Outside a transaction block SET LOCAL has no effect.
            if self.in_transaction {
                self.lock_timeout_local = enabled;
            }
        } else {
            self.lock_timeout_session = enabled;
        }
    }

    fn mark_created(&mut self, relation: Option<&RangeVar>) {
        if let Some(relation) = relation {
            self.created_tables
                .insert(TableRef::from_range_var(relation));
        }
    }
}

/// COMMIT, ROLLBACK (with or without AND CHAIN) and PREPARE TRANSACTION.
pub fn ends_transaction(node: &NodeEnum) -> bool {
    let NodeEnum::TransactionStmt(stmt) = node else {
        return false;
    };
    [
        TransactionStmtKind::TransStmtCommit,
        TransactionStmtKind::TransStmtRollback,
        TransactionStmtKind::TransStmtPrepare,
    ]
    .iter()
    .any(|k| *k as i32 == stmt.kind)
}

/// `0`, `'0'`, `'0ms'`, `'0s'`: a lock_timeout of zero means "wait forever".
fn is_zero(value: &Node) -> bool {
    let Some(NodeEnum::AConst(c)) = value.node.as_ref() else {
        return false;
    };
    match &c.val {
        Some(a_const::Val::Ival(i)) => i.ival == 0,
        Some(a_const::Val::Sval(s)) => {
            let number: String = s
                .sval
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            number.parse::<f64>().is_ok_and(|n| n == 0.0)
        }
        _ => false,
    }
}
