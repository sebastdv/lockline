//! What the analyzer knows while walking one migration unit.

use std::collections::HashSet;

use pg_query::NodeEnum;
use pg_query::protobuf::{RangeVar, TransactionStmtKind};

use crate::analyze::TransactionMode;
use crate::model::TableRef;

#[derive(Debug, Clone)]
pub struct UnitContext {
    /// Tables created earlier in this unit: they have no rows and no traffic yet.
    pub created_tables: HashSet<TableRef>,
    pub in_transaction: bool,
}

impl UnitContext {
    pub fn new(mode: TransactionMode) -> Self {
        UnitContext {
            created_tables: HashSet::new(),
            in_transaction: mode == TransactionMode::PerUnit,
        }
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
            NodeEnum::SelectStmt(stmt) => {
                self.mark_created(stmt.into_clause.as_ref().and_then(|i| i.rel.as_ref()));
            }
            NodeEnum::TransactionStmt(stmt) => {
                let kind = stmt.kind;
                if kind == TransactionStmtKind::TransStmtBegin as i32
                    || kind == TransactionStmtKind::TransStmtStart as i32
                {
                    self.in_transaction = true;
                } else if kind == TransactionStmtKind::TransStmtCommit as i32
                    || kind == TransactionStmtKind::TransStmtRollback as i32
                {
                    self.in_transaction = false;
                }
            }
            _ => {}
        }
    }

    fn mark_created(&mut self, relation: Option<&RangeVar>) {
        if let Some(relation) = relation {
            self.created_tables
                .insert(TableRef::from_range_var(relation));
        }
    }
}
