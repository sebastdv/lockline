use pg_query::NodeEnum;

use super::{Rule, RuleCtx};
use crate::model::{Analysis, Cost, Finding, Severity};

pub struct AddColumnRewrite;

impl Rule for AddColumnRewrite {
    fn id(&self) -> &'static str {
        "add-column-rewrite"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        if ctx.new_table || !matches!(ctx.stmt.node, NodeEnum::AlterTableStmt(_)) {
            return None;
        }
        let Analysis::Locks(effects) = ctx.analysis else {
            return None;
        };
        let effect = effects.iter().find(|e| e.cost == Cost::Rewrite)?;
        Some(self.finding(
            format!(
                "Adding this column rewrites \"{}\" and all its indexes while holding an ACCESS EXCLUSIVE lock.",
                effect.table
            ),
            Some(
                "Add the column without a default (or with a constant default), then backfill in batches and set the default afterwards."
                    .to_string(),
            ),
        ))
    }
}
